mod identity;
#[path = "sessions/support.rs"]
pub mod support;

use std::sync::{Arc, Mutex};

use lens_contract::auth::{Identity, Role};
use lens_server::tracing::{TraceAgent, TraceConfig, TraceReadError, TraceReader};
use litellm_traces::{
    QueryScope, SpanDetail, SpanErrorPage, Trace, TracePage,
    query::named::ReadAccessParams,
    request::{TraceDetailRequest, TraceErrorPageRequest, TraceSpanRequest},
};
use rstest::rstest;
use serde_json::{Value, json};
use support::{ADMIN, Database, Server, database};

#[derive(Clone, Copy)]
enum Failure {
    Invalid,
    Changed,
    Large,
    Unavailable,
}

#[derive(Debug)]
struct Call {
    scope: ReadAccessParams,
    parameters: Value,
}

#[derive(Clone, Default)]
struct Reader {
    calls: Arc<Mutex<Vec<Call>>>,
    failure: Option<Failure>,
}

impl Reader {
    fn record(&self, scope: ReadAccessParams, parameters: Value) -> Result<(), TraceReadError> {
        self.calls.lock().unwrap().push(Call { scope, parameters });
        match self.failure {
            None => Ok(()),
            Some(Failure::Invalid) => {
                Err(TraceReadError::InvalidRequest("Invalid list cursor".into()))
            }
            Some(Failure::Changed) => Err(TraceReadError::Changed),
            Some(Failure::Large) => Err(TraceReadError::TooLarge),
            Some(Failure::Unavailable) => Err(TraceReadError::Unavailable),
        }
    }
}

fn read_scope(scope: QueryScope) -> ReadAccessParams {
    match scope {
        QueryScope::All => ReadAccessParams {
            all_teams: true,
            user_id: String::new(),
            team_ids: Vec::new(),
        },
        QueryScope::Owned { user_id, team_ids } => ReadAccessParams {
            all_teams: false,
            user_id,
            team_ids,
        },
    }
}

impl TraceReader for Reader {
    type Help = Value;
    async fn list(
        &self,
        scope: ReadAccessParams,
        start_ms: i64,
        end_ms: i64,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<TracePage, TraceReadError> {
        self.record(
            scope,
            json!({"start_ms":start_ms,"end_ms":end_ms,"cursor":cursor,"limit":limit}),
        )?;
        Ok(TracePage {
            data: Vec::new(),
            next_cursor: None,
        })
    }
    async fn agents(
        &self,
        scope: ReadAccessParams,
        start_ms: i64,
        end_ms: i64,
        limit: u32,
    ) -> Result<Vec<TraceAgent>, TraceReadError> {
        self.record(
            scope,
            json!({"start_ms":start_ms,"end_ms":end_ms,"limit":limit}),
        )?;
        Ok(Vec::new())
    }
    async fn trace(
        &self,
        scope: ReadAccessParams,
        trace_id: String,
        request: TraceDetailRequest,
    ) -> Result<Option<Trace>, TraceReadError> {
        self.record(scope, json!({"trace_id":trace_id,"trace_ref":request.trace_ref,"cursor":request.cursor,"page_size":request.page_size}))?;
        Ok(None)
    }
    async fn span(
        &self,
        scope: ReadAccessParams,
        trace_id: String,
        span_id: String,
        request: TraceSpanRequest,
    ) -> Result<Option<SpanDetail>, TraceReadError> {
        self.record(
            scope,
            json!({"trace_id":trace_id,"span_id":span_id,"trace_ref":request.trace_ref}),
        )?;
        Ok(None)
    }
    async fn span_error(
        &self,
        scope: ReadAccessParams,
        trace_id: String,
        span_id: String,
        request: TraceErrorPageRequest,
    ) -> Result<Option<SpanErrorPage>, TraceReadError> {
        self.record(scope, json!({"trace_id":trace_id,"span_id":span_id,"trace_ref":request.trace_ref,"cursor":request.cursor}))?;
        Ok(None)
    }
    async fn query(&self, scope: QueryScope, sql: String) -> Result<Value, TraceReadError> {
        self.record(read_scope(scope), json!({"sql":sql}))?;
        Ok(json!({"data":[{"result":1}]}))
    }
    async fn help(&self, scope: QueryScope) -> Result<Value, TraceReadError> {
        self.record(read_scope(scope), json!({}))?;
        Ok(json!({"guide":"scoped"}))
    }
}

async fn serve(database: &Database, reader: Reader) -> Server {
    database
        .serve_router(false, |auth| {
            lens_server::sessions::router_with_auth(auth.clone()).merge(
                lens_server::tracing::router(auth, reader, TraceConfig::default()),
            )
        })
        .await
}

#[rstest]
#[case::list("GET", "/v1/traces", 200)]
#[case::agents("GET", "/v1/traces/agents", 200)]
#[case::trace("GET", "/v1/traces/trace-id", 404)]
#[case::span("GET", "/v1/traces/trace-id/spans/span-id", 404)]
#[case::error("GET", "/v1/traces/trace-id/spans/span-id/error", 404)]
#[case::sql("POST", "/v1/traces/query", 200)]
#[case::help("GET", "/v1/traces/query/help", 200)]
#[tokio::test]
async fn public_reads_use_signed_identity_scope_and_ignore_browser_scope(
    #[future(awt)] database: Database,
    #[case] method: &str,
    #[case] path: &str,
    #[case] status: u16,
) {
    let reader = Reader::default();
    let server = serve(&database, reader.clone()).await;
    let token = identity::delegated(Identity {
        user_id: Some("user-a".into()),
        team_id: Some("primary-team".into()),
        log_team_ids: vec!["log-team".into()],
        ..Identity::default()
    });
    let response = server
        .client
        .request(method.parse().unwrap(), server.url.join(path).unwrap())
        .bearer_auth(token)
        .query(&[
            ("all_teams", "1"),
            ("user_id", "other-user"),
            ("team_ids", "other-team"),
        ])
        .json(&json!({"sql":"SELECT 1 AS result"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), status);
    let calls = reader.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(!calls[0].scope.all_teams);
    assert_eq!(calls[0].scope.user_id, "user-a");
    assert_eq!(calls[0].scope.team_ids, vec!["log-team"]);
}

#[rstest]
#[case::list("GET", "/v1/traces", "Not allowed to view agent traces")]
#[case::sql("POST", "/v1/traces/query", "Not allowed to view logs")]
#[tokio::test]
async fn identity_without_a_read_scope_cannot_reach_storage(
    #[future(awt)] database: Database,
    #[case] method: &str,
    #[case] path: &str,
    #[case] detail: &str,
) {
    let reader = Reader::default();
    let server = serve(&database, reader.clone()).await;
    let token = identity::delegated(Identity {
        token: Some("key-hash".into()),
        ..Identity::default()
    });
    let response = server
        .client
        .request(method.parse().unwrap(), server.url.join(path).unwrap())
        .bearer_auth(token)
        .json(&json!({"sql":"SELECT 1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"detail":detail})
    );
    assert!(reader.calls.lock().unwrap().is_empty());
}

#[rstest]
#[case::admin(Role::ProxyAdmin)]
#[case::viewer(Role::ProxyAdminViewer)]
#[tokio::test]
async fn admin_reads_use_all_rows_and_preserve_supplied_request_parameters(
    #[future(awt)] database: Database,
    #[case] role: Role,
) {
    let reader = Reader::default();
    let server = serve(&database, reader.clone()).await;
    let token = identity::delegated(Identity {
        user_id: Some("user-a".into()),
        user_role: role,
        ..Identity::default()
    });
    let response = server
        .client
        .get(
            server
                .url
                .join("/v1/traces?start_ms=1&end_ms=2&cursor=opaque")
                .unwrap(),
        )
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"data":[],"next_cursor":null})
    );
    let calls = reader.calls.lock().unwrap();
    assert!(calls[0].scope.all_teams);
    assert!(calls[0].scope.user_id.is_empty());
    assert!(calls[0].scope.team_ids.is_empty());
    assert_eq!(
        calls[0].parameters,
        json!({"start_ms":1,"end_ms":2,"cursor":"opaque","limit":50})
    );
}

#[rstest]
#[case::invalid(Failure::Invalid, 400, "invalid_request", "Invalid list cursor", None)]
#[case::changed(
    Failure::Changed,
    409,
    "trace_changed",
    "Trace changed while paging; refresh the trace to continue",
    None
)]
#[case::large(
    Failure::Large,
    413,
    "too_large",
    "Trace is too large for this view. Use a filtered trace query.",
    None
)]
#[case::unavailable(
    Failure::Unavailable,
    503,
    "unavailable",
    "Traces are temporarily unavailable. Please try again.",
    Some("2")
)]
#[tokio::test]
async fn read_failures_preserve_public_status_details_and_retry_headers(
    #[future(awt)] database: Database,
    #[case] failure: Failure,
    #[case] status: u16,
    #[case] code: &str,
    #[case] message: &str,
    #[case] retry: Option<&str>,
) {
    let server = serve(
        &database,
        Reader {
            failure: Some(failure),
            ..Reader::default()
        },
    )
    .await;
    let response = server
        .client
        .get(server.url.join("/v1/traces").unwrap())
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), status);
    assert_eq!(
        response
            .headers()
            .get("retry-after")
            .map(|value| value.to_str().unwrap()),
        retry
    );
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"detail":{"code":code,"message":message}})
    );
}

#[rstest]
#[case::list("/v1/traces?start_ms=bad&end_ms=bad", json!(["query","start_ms"]), "int_parsing")]
#[case::small_page("/v1/traces/id?page_size=0", json!(["query","page_size"]), "greater_than_equal")]
#[case::large_page("/v1/traces/id?page_size=501", json!(["query","page_size"]), "less_than_equal")]
#[case::wide_page("/v1/traces/id?page_size=999999999999999999999999", json!(["query","page_size"]), "less_than_equal")]
#[tokio::test]
async fn invalid_query_never_reaches_reader(
    #[future(awt)] database: Database,
    #[case] path: &str,
    #[case] loc: Value,
    #[case] kind: &str,
) {
    let reader = Reader::default();
    let server = serve(&database, reader.clone()).await;
    let response = server
        .client
        .get(server.url.join(path).unwrap())
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 422);
    let errors: Value = response.json().await.unwrap();
    assert_eq!(errors["detail"][0]["type"], kind);
    assert_eq!(errors["detail"][0]["loc"], loc);
    assert!(reader.calls.lock().unwrap().is_empty());
}

#[rstest]
#[case::invalid_sql(
    "POST",
    "/v1/traces/query",
    Failure::Invalid,
    400,
    "Invalid list cursor"
)]
#[case::unavailable_sql(
    "POST",
    "/v1/traces/query",
    Failure::Unavailable,
    503,
    "Trace SQL query failed or exceeded reader limits"
)]
#[case::unavailable_help(
    "GET",
    "/v1/traces/query/help",
    Failure::Unavailable,
    503,
    "Trace query help is temporarily unavailable"
)]
#[tokio::test]
async fn sql_and_help_keep_their_separate_error_envelopes(
    #[future(awt)] database: Database,
    #[case] method: &str,
    #[case] path: &str,
    #[case] failure: Failure,
    #[case] status: u16,
    #[case] message: &str,
) {
    let server = serve(
        &database,
        Reader {
            failure: Some(failure),
            ..Reader::default()
        },
    )
    .await;
    let response = server
        .client
        .request(method.parse().unwrap(), server.url.join(path).unwrap())
        .bearer_auth(ADMIN)
        .json(&json!({"sql":"SELECT 1"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), status);
    assert!(!response.headers().contains_key("retry-after"));
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"detail":message})
    );
}

#[rstest]
#[case::list("/v1/traces/?start_ms=1", "/v1/traces?start_ms=1")]
#[case::agents("/v1/traces/agents/", "/v1/traces/agents")]
#[case::help("/v1/traces/query/help/", "/v1/traces/query/help")]
#[case::span("/v1/traces/id/spans/span/", "/v1/traces/id/spans/span")]
#[tokio::test]
async fn trace_trailing_slashes_preserve_redirect_location(
    #[future(awt)] database: Database,
    #[case] path: &str,
    #[case] target: &str,
) {
    let reader = Reader::default();
    let server = serve(&database, reader.clone()).await;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let response = client
        .get(server.url.join(path).unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 307);
    assert_eq!(
        response.headers()["location"],
        server.url.join(target).unwrap().as_str()
    );
    assert!(reader.calls.lock().unwrap().is_empty());
}
