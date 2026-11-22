mod identity;
#[path = "sessions/support.rs"]
pub mod support;

use std::sync::{Arc, Mutex};

use lens_contract::auth::{Identity, Role};
use lens_server::tracing::{TraceAgent, TraceConfig, TraceReadError, TraceReader};
use litellm_traces::{
    LogListRequest, OtelLogPage, OtelLogRecord, QueryScope, SpanDetail, SpanErrorPage, Trace,
    TraceConversationPage, TracePage,
    query::named::ReadAccessParams,
    request::{
        TraceConversationRequest, TraceDetailRequest, TraceErrorPageRequest, TraceSpanRequest,
    },
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
    conversation_found: bool,
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
        agent: String,
        include_internal: bool,
        start_ms: i64,
        end_ms: i64,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<TracePage, TraceReadError> {
        self.record(
            scope,
            json!({"start_ms":start_ms,"end_ms":end_ms,"cursor":cursor,"limit":limit,"agent":agent,"include_internal":include_internal}),
        )?;
        Ok(TracePage {
            data: Vec::new(),
            next_cursor: None,
        })
    }
    async fn logs(
        &self,
        scope: QueryScope,
        request: LogListRequest,
    ) -> Result<OtelLogPage, TraceReadError> {
        self.record(
            read_scope(scope),
            json!({
                "start_ms":request.start_ms,
                "end_ms":request.end_ms,
                "service":request.service,
                "event":request.event,
                "severity":request.severity,
                "trace_id":request.trace_id,
                "conversation_id":request.conversation_id,
                "cursor":request.cursor,
                "page_size":request.page_size,
            }),
        )?;
        Ok(OtelLogPage {
            data: Vec::new(),
            next_cursor: None,
        })
    }
    async fn log(
        &self,
        scope: QueryScope,
        record_id: String,
    ) -> Result<Option<OtelLogRecord>, TraceReadError> {
        self.record(read_scope(scope), json!({"record_id":record_id}))?;
        Ok(None)
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
    async fn conversation(
        &self,
        scope: ReadAccessParams,
        trace_id: String,
        request: TraceConversationRequest,
    ) -> Result<Option<TraceConversationPage>, TraceReadError> {
        self.record(scope, json!({"trace_id":trace_id,"trace_ref":request.trace_ref,"cursor":request.cursor,"page_size":request.page_size}))?;
        Ok(self.conversation_found.then(|| TraceConversationPage {
            turns: Vec::new(),
            next_cursor: Some("following".into()),
        }))
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
#[case::under_limit(0, 401)]
#[case::over_limit(lens_server::routing::MAX_REQUEST_BYTES, 413)]
#[tokio::test]
async fn unauthenticated_queries_enforce_body_limits_before_storage(
    #[case] padding: usize,
    #[case] status: u16,
) {
    use axum::{body::Body, http::Request};
    use lens_auth::{Authentication, Settings};
    use litellm_storage_clickhouse::{Connection, sessions::Sessions, state::ClickHouseState};
    use tower::ServiceExt;

    let authentication = Arc::new(Authentication {
        settings: Settings::new(ADMIN, None, "http://localhost").unwrap(),
        sessions: Sessions(ClickHouseState::new(
            litellm_http::Client::no_redirect_for_test(),
            Connection::reader("http://127.0.0.1:9", "unused").unwrap(),
        )),
    });
    let reader = Reader::default();
    let app = lens_server::tracing::router(authentication, reader.clone(), TraceConfig::default());
    let response = app
        .oneshot(
            Request::post("/v1/traces/query")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    "{}{{\"sql\":\"SELECT 1\"}}",
                    " ".repeat(padding)
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), status);
    assert!(reader.calls.lock().unwrap().is_empty());
}

#[rstest]
#[case::list("/v1/traces", 200, true)]
#[case::detail("/v1/traces/trace-id", 404, true)]
#[case::span("/v1/traces/trace-id/spans/span-id", 404, true)]
#[case::unneeded_route("/v1/traces/agents", 401, false)]
#[tokio::test]
async fn slack_credential_cannot_expand_trace_scope(
    #[case] path: &str,
    #[case] status: u16,
    #[case] reads: bool,
) {
    use axum::{body::Body, http::Request};
    use lens_auth::{Authentication, Settings};
    use litellm_storage_clickhouse::{Connection, sessions::Sessions, state::ClickHouseState};
    use tower::ServiceExt;

    const KEY: &str = "slack-investigator-test-token-only";
    let authentication = Arc::new(Authentication {
        settings: Settings::new(ADMIN, None, "http://localhost")
            .unwrap()
            .with_slack_investigator(KEY, "docs-team")
            .unwrap(),
        sessions: Sessions(ClickHouseState::new(
            litellm_http::Client::no_redirect_for_test(),
            Connection::reader("http://127.0.0.1:9", "unused").unwrap(),
        )),
    });
    let reader = Reader::default();
    let app = lens_server::tracing::router(authentication, reader.clone(), TraceConfig::default());
    let response = app
        .oneshot(
            Request::get(format!(
                "{path}?all_teams=1&team_ids=other-team&user_id=other-user"
            ))
            .header("authorization", format!("Bearer {KEY}"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), status);
    let calls = reader.calls.lock().unwrap();
    if reads {
        assert_eq!(calls.len(), 1);
        assert!(!calls[0].scope.all_teams);
        assert!(calls[0].scope.user_id.is_empty());
        assert_eq!(calls[0].scope.team_ids, vec!["docs-team"]);
    } else {
        assert!(calls.is_empty());
    }
}

#[rstest]
#[case::list("GET", "/v1/traces", 200)]
#[case::logs("GET", "/v1/logs", 200)]
#[case::lens_logs("GET", "/lens/logs", 200)]
#[case::agents("GET", "/v1/traces/agents", 200)]
#[case::trace("GET", "/v1/traces/trace-id", 404)]
#[case::conversation("GET", "/v1/traces/trace-id/conversation", 404)]
#[case::span("GET", "/v1/traces/trace-id/spans/span-id", 404)]
#[case::error("GET", "/v1/traces/trace-id/spans/span-id/error", 404)]
#[case::log_detail(
    "GET",
    "/v1/logs/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    404
)]
#[case::lens_log_detail(
    "GET",
    "/lens/logs/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    404
)]
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
#[case::logs("GET", "/v1/logs", "Not allowed to view logs")]
#[case::lens_logs("GET", "/lens/logs", "Not allowed to view logs")]
#[case::conversation(
    "GET",
    "/v1/traces/id/conversation",
    "Not allowed to view agent traces"
)]
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
#[tokio::test]
async fn conversation_requires_authentication_before_reading_storage(
    #[future(awt)] database: Database,
    #[values("/v1/traces/id/conversation", "/lens/traces/id/conversation")] path: &str,
) {
    let reader = Reader::default();
    let server = serve(&database, reader.clone()).await;
    let response = server
        .client
        .get(server.url.join(path).unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    assert!(reader.calls.lock().unwrap().is_empty());
}

#[rstest]
#[tokio::test]
async fn embedded_conversation_uses_delegated_scope_and_preserves_paging(
    #[future(awt)] database: Database,
) {
    let reader = Reader {
        conversation_found: true,
        ..Reader::default()
    };
    let server = serve(&database, reader.clone()).await;
    let token = identity::delegated(Identity {
        user_id: Some("user-a".into()),
        user_role: Role::InternalUser,
        log_team_ids: vec!["team-a".into()],
        ..Identity::default()
    });
    let response = server
        .client
        .get(
            server
                .url
                .join("/lens/traces/id/conversation?trace_ref=ref%2Bone&cursor=next&page_size=3")
                .unwrap(),
        )
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"turns":[],"next_cursor":"following"})
    );
    let calls = reader.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(!calls[0].scope.all_teams);
    assert_eq!(calls[0].scope.user_id, "user-a");
    assert_eq!(calls[0].scope.team_ids, ["team-a"]);
    assert_eq!(
        calls[0].parameters,
        json!({"trace_id":"id","trace_ref":"ref+one","cursor":"next","page_size":3})
    );
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
                .join("/v1/traces?start_ms=1&end_ms=2&cursor=opaque&agent=A")
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
        json!({"start_ms":1,"end_ms":2,"cursor":"opaque","limit":50,"agent":"A","include_internal":false})
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
    #[values("/v1/traces", "/v1/traces/id/conversation")] path: &str,
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
        .get(server.url.join(path).unwrap())
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
#[case::list_internal("/v1/traces?include_internal=maybe", json!(["query","include_internal"]), "bool_parsing")]
#[case::small_page("/v1/traces/id?page_size=0", json!(["query","page_size"]), "greater_than_equal")]
#[case::conversation_small_page("/v1/traces/id/conversation?page_size=0", json!(["query","page_size"]), "greater_than_equal")]
#[case::conversation_large_page("/v1/traces/id/conversation?page_size=51", json!(["query","page_size"]), "less_than_equal")]
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
#[case::omitted("/v1/traces?start_ms=1&end_ms=2", false)]
#[case::off("/v1/traces?start_ms=1&end_ms=2&include_internal=false", false)]
#[case::on("/v1/traces?start_ms=1&end_ms=2&include_internal=true", true)]
#[tokio::test]
async fn list_shows_internal_traces_only_when_asked(
    #[future(awt)] database: Database,
    #[case] path: &str,
    #[case] include_internal: bool,
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
    assert_eq!(response.status(), 200);
    let calls = reader.calls.lock().unwrap();
    assert_eq!(calls[0].parameters["include_internal"], include_internal);
}

#[rstest]
#[tokio::test]
async fn overlong_agent_filter_is_rejected_before_reading(#[future(awt)] database: Database) {
    let reader = Reader::default();
    let server = serve(&database, reader.clone()).await;
    let response = server
        .client
        .get(server.url.join("/v1/traces").unwrap())
        .query(&[("agent", "A".repeat(257))])
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 422);
    let errors: Value = response.json().await.unwrap();
    assert_eq!(errors["detail"][0]["type"], "string_too_long");
    assert_eq!(errors["detail"][0]["loc"], json!(["query", "agent"]));
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

async fn mcp(server: &Server, token: Option<&str>, body: Value) -> reqwest::Response {
    let request = server
        .client
        .post(server.url.join("/lens/mcp").unwrap())
        .json(&body);
    match token {
        Some(token) => request.bearer_auth(token),
        None => request,
    }
    .send()
    .await
    .unwrap()
}

fn tool_call(name: &str, arguments: Value) -> Value {
    json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":name,"arguments":arguments}})
}

#[rstest]
#[case::list_agents("list_agents", json!({}), json!({"limit":500}))]
#[case::list_traces(
    "list_traces",
    json!({"agent":"A","cursor":"opaque","include_internal":true}),
    json!({"cursor":"opaque","agent":"A","include_internal":true,"limit":50})
)]
#[case::get_trace(
    "get_trace",
    json!({"trace_id":"t","trace_ref":"r","cursor":"c","page_size":5}),
    json!({"trace_id":"t","trace_ref":"r","cursor":"c","page_size":5})
)]
#[case::get_trace_default_page(
    "get_trace",
    json!({"trace_id":"t"}),
    json!({"trace_id":"t","trace_ref":"","cursor":null,"page_size":50})
)]
#[case::get_trace_conversation(
    "get_trace_conversation",
    json!({"trace_id":"t"}),
    json!({"trace_id":"t","trace_ref":"","cursor":null,"page_size":10})
)]
#[case::get_span(
    "get_span",
    json!({"trace_id":"t","span_id":"s","trace_ref":"r"}),
    json!({"trace_id":"t","span_id":"s","trace_ref":"r"})
)]
#[tokio::test]
async fn mcp_tools_read_with_the_signed_identity_scope(
    #[future(awt)] database: Database,
    #[case] name: &str,
    #[case] arguments: Value,
    #[case] expected: Value,
) {
    let reader = Reader::default();
    let server = serve(&database, reader.clone()).await;
    let token = identity::delegated(Identity {
        user_id: Some("user-a".into()),
        team_id: Some("primary-team".into()),
        log_team_ids: vec!["log-team".into()],
        ..Identity::default()
    });
    let response = mcp(&server, Some(&token), tool_call(name, arguments)).await;
    assert_eq!(response.status(), 200);
    let body = response.json::<Value>().await.unwrap();
    assert_eq!(body["id"], 7);
    assert_eq!(body["result"]["content"][0]["type"], "text");
    let calls = reader.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(!calls[0].scope.all_teams);
    assert_eq!(calls[0].scope.user_id, "user-a");
    assert_eq!(calls[0].scope.team_ids, vec!["log-team"]);
    let recorded = calls[0].parameters.as_object().unwrap();
    assert!(
        expected
            .as_object()
            .unwrap()
            .iter()
            .all(|(key, value)| recorded.get(key) == Some(value)),
        "{recorded:?}"
    );
}

#[rstest]
#[tokio::test]
async fn mcp_batch_answers_each_tool_call_in_order(#[future(awt)] database: Database) {
    let reader = Reader::default();
    let server = serve(&database, reader.clone()).await;
    let body = mcp(
        &server,
        Some(ADMIN),
        json!([
            {"jsonrpc":"2.0","id":"a","method":"tools/call","params":{"name":"list_traces","arguments":{}}},
            {"jsonrpc":"2.0","id":"b","method":"tools/call","params":{"name":"list_agents","arguments":{}}}
        ]),
    )
    .await
    .json::<Value>()
    .await
    .unwrap();
    assert_eq!(
        (&body[0]["id"], &body[0]["result"]["isError"]),
        (&json!("a"), &json!(false))
    );
    assert_eq!(
        (&body[1]["id"], &body[1]["result"]["isError"]),
        (&json!("b"), &json!(false))
    );
    assert_eq!(body.as_array().unwrap().len(), 2);
    assert_eq!(reader.calls.lock().unwrap().len(), 2);
}

#[rstest]
#[tokio::test]
async fn mcp_admin_token_reads_all_rows_and_returns_the_page_as_text(
    #[future(awt)] database: Database,
) {
    let reader = Reader::default();
    let server = serve(&database, reader.clone()).await;
    let response = mcp(
        &server,
        Some(ADMIN),
        tool_call("list_traces", json!({"hours":2})),
    )
    .await;
    let body = response.json::<Value>().await.unwrap();
    assert_eq!(body["result"]["isError"], false);
    assert_eq!(
        serde_json::from_str::<Value>(body["result"]["content"][0]["text"].as_str().unwrap())
            .unwrap(),
        json!({"data":[],"next_cursor":null})
    );
    let calls = reader.calls.lock().unwrap();
    assert!(calls[0].scope.all_teams);
    let window = calls[0].parameters["end_ms"].as_i64().unwrap()
        - calls[0].parameters["start_ms"].as_i64().unwrap();
    assert_eq!(window, 2 * 3_600_000);
}

#[rstest]
#[case::missing_token(None, 401)]
#[case::unknown_token(Some("sk-not-a-lens-credential"), 401)]
#[tokio::test]
async fn mcp_rejects_callers_lens_cannot_identify(
    #[future(awt)] database: Database,
    #[case] token: Option<&str>,
    #[case] status: u16,
) {
    let reader = Reader::default();
    let server = serve(&database, reader.clone()).await;
    let response = mcp(&server, token, tool_call("list_traces", json!({}))).await;
    assert_eq!(response.status(), status);
    assert!(reader.calls.lock().unwrap().is_empty());
}

#[rstest]
#[tokio::test]
async fn mcp_identity_without_a_read_scope_cannot_reach_storage(#[future(awt)] database: Database) {
    let reader = Reader::default();
    let server = serve(&database, reader.clone()).await;
    let token = identity::delegated(Identity {
        token: Some("key-hash".into()),
        ..Identity::default()
    });
    let response = mcp(&server, Some(&token), tool_call("list_traces", json!({}))).await;
    assert_eq!(response.status(), 403);
    assert!(reader.calls.lock().unwrap().is_empty());
}

#[rstest]
#[case::unknown_tool("delete_trace", json!({}), "Unknown tool delete_trace")]
#[case::unknown_argument("list_traces", json!({"user_id":"other-user"}), "Invalid arguments")]
#[case::missing_trace_id("get_trace", json!({}), "Invalid arguments")]
#[case::page_too_large("get_trace", json!({"trace_id":"t","page_size":501}), "page_size must be between 1 and 500")]
#[case::conversation_page_too_large(
    "get_trace_conversation",
    json!({"trace_id":"t","page_size":51}),
    "page_size must be between 1 and 50"
)]
#[tokio::test]
async fn mcp_refuses_bad_tool_calls_before_reading(
    #[future(awt)] database: Database,
    #[case] name: &str,
    #[case] arguments: Value,
    #[case] message: &str,
) {
    let reader = Reader::default();
    let server = serve(&database, reader.clone()).await;
    let response = mcp(&server, Some(ADMIN), tool_call(name, arguments)).await;
    assert_eq!(response.status(), 200);
    let body = response.json::<Value>().await.unwrap();
    assert_eq!(body["result"]["isError"], true);
    assert!(
        body["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with(message),
        "{body}"
    );
    assert!(reader.calls.lock().unwrap().is_empty());
}

#[rstest]
#[case::missing_trace("get_trace", json!({"trace_id":"t"}), None, "Trace not found")]
#[case::missing_span("get_span", json!({"trace_id":"t","span_id":"s"}), None, "Span not found")]
#[case::storage_down(
    "list_traces",
    json!({}),
    Some(Failure::Unavailable),
    "Traces are temporarily unavailable. Please try again."
)]
#[tokio::test]
async fn mcp_reports_read_failures_as_tool_errors(
    #[future(awt)] database: Database,
    #[case] name: &str,
    #[case] arguments: Value,
    #[case] failure: Option<Failure>,
    #[case] message: &str,
) {
    let reader = Reader {
        failure,
        ..Reader::default()
    };
    let server = serve(&database, reader).await;
    let body = mcp(&server, Some(ADMIN), tool_call(name, arguments))
        .await
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(
        body["result"],
        json!({"content":[{"type":"text","text":message}],"isError":true})
    );
}

#[rstest]
#[case::current("2025-06-18", "2025-06-18")]
#[case::older("2024-11-05", "2024-11-05")]
#[case::unknown("1999-01-01", "2025-06-18")]
#[tokio::test]
async fn mcp_initialize_negotiates_the_protocol_version(
    #[future(awt)] database: Database,
    #[case] requested: &str,
    #[case] agreed: &str,
) {
    let server = serve(&database, Reader::default()).await;
    let body = mcp(
        &server,
        Some(ADMIN),
        json!({"jsonrpc":"2.0","id":"a","method":"initialize","params":{"protocolVersion":requested}}),
    )
    .await
    .json::<Value>()
    .await
    .unwrap();
    assert_eq!(body["id"], "a");
    assert_eq!(body["result"]["protocolVersion"], agreed);
    assert_eq!(body["result"]["capabilities"], json!({"tools":{}}));
}

#[rstest]
#[tokio::test]
async fn mcp_lists_only_read_only_tools(#[future(awt)] database: Database) {
    let server = serve(&database, Reader::default()).await;
    let body = mcp(
        &server,
        Some(ADMIN),
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
    )
    .await
    .json::<Value>()
    .await
    .unwrap();
    let tools = body["result"]["tools"].as_array().unwrap();
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "list_agents",
            "list_traces",
            "get_trace",
            "get_trace_conversation",
            "get_span"
        ]
    );
    assert!(
        tools
            .iter()
            .all(|tool| tool["annotations"]["readOnlyHint"] == true)
    );
}

#[rstest]
#[case::notification(json!({"jsonrpc":"2.0","method":"notifications/initialized"}), 202, None)]
#[case::ping(json!({"jsonrpc":"2.0","id":2,"method":"ping"}), 200, Some(json!({"jsonrpc":"2.0","id":2,"result":{}})))]
#[case::unknown_method(
    json!({"jsonrpc":"2.0","id":3,"method":"resources/list"}),
    200,
    Some(json!({"jsonrpc":"2.0","id":3,"error":{"code":-32601,"message":"Method not found"}}))
)]
#[case::batch(
    json!([
        {"jsonrpc":"2.0","id":4,"method":"ping"},
        {"jsonrpc":"2.0","method":"notifications/initialized"},
        {"jsonrpc":"2.0","id":5,"method":"resources/list"},
        7
    ]),
    200,
    Some(json!([
        {"jsonrpc":"2.0","id":4,"result":{}},
        {"jsonrpc":"2.0","id":5,"error":{"code":-32601,"message":"Method not found"}},
        {"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Invalid JSON-RPC request"}}
    ]))
)]
#[case::notification_batch(json!([{"jsonrpc":"2.0","method":"notifications/initialized"}]), 202, None)]
#[case::empty_batch(
    json!([]),
    200,
    Some(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Invalid JSON-RPC request"}}))
)]
#[case::oversized_batch(
    Value::Array(vec![json!({"jsonrpc":"2.0","id":1,"method":"ping"}); 17]),
    200,
    Some(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Invalid JSON-RPC request"}}))
)]
#[case::not_a_request(
    json!("ping"),
    200,
    Some(json!({"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"Invalid JSON-RPC request"}}))
)]
#[tokio::test]
async fn mcp_answers_protocol_messages_without_reading(
    #[future(awt)] database: Database,
    #[case] message: Value,
    #[case] status: u16,
    #[case] expected: Option<Value>,
) {
    let reader = Reader::default();
    let server = serve(&database, reader.clone()).await;
    let response = mcp(&server, Some(ADMIN), message).await;
    assert_eq!(response.status(), status);
    let text = response.text().await.unwrap();
    assert_eq!(
        (!text.is_empty()).then(|| serde_json::from_str::<Value>(&text).unwrap()),
        expected
    );
    assert!(reader.calls.lock().unwrap().is_empty());
}
