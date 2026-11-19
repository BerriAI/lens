mod identity;
#[path = "sessions/support.rs"]
pub mod support;

use std::sync::{Arc, Mutex};

use lens_contract::{
    auth::{Identity, Role},
    profiles::ModelCapabilities,
};
use lens_server::profiles::{
    ConfiguredModel, ProfileStore, ProfileStoreError, RunQuery,
    aggregate::{Call, Charge, Column, Run, RunCalls, run_key},
};
use litellm_traces::{LedgerRequest, query::named::ReadAccessParams};
use reqwest::{Method, StatusCode};
use rstest::{fixture, rstest};
use serde_json::{Value, json};
use support::{Database, Server, database};

const AGENT: &str = "assistant";

#[derive(Clone, Default)]
struct Memory {
    queries: Arc<Mutex<Vec<RunQuery>>>,
    runs: Vec<Run>,
    calls: RunCalls,
    unavailable: bool,
}

impl ProfileStore for Memory {
    async fn runs(
        &self,
        _: ReadAccessParams,
        query: RunQuery,
    ) -> Result<Vec<Run>, ProfileStoreError> {
        self.queries.lock().expect("queries").push(query);
        if self.unavailable {
            return Err(ProfileStoreError::Unavailable);
        }
        Ok(self.runs.clone())
    }

    async fn calls(
        &self,
        _: ReadAccessParams,
        _: Vec<Run>,
        _: usize,
    ) -> Result<RunCalls, ProfileStoreError> {
        Ok(self.calls.clone())
    }

    async fn configured_models(&self) -> Vec<ConfiguredModel> {
        vec![ConfiguredModel {
            model_id: "dep-text".into(),
            model_group: "text-only".into(),
            provider: "openai".into(),
            capabilities: ModelCapabilities {
                function_calling: Some(true),
                vision: Some(false),
                reasoning: None,
                structured_output: None,
            },
        }]
    }
}

fn run(trace: &str, profile_id: &str, category: &str) -> Run {
    Run {
        team_id: "team".into(),
        api_key_hash: "key".into(),
        trace_id: trace.into(),
        start_ms: 10,
        end_ms: 10,
        profile_id: profile_id.into(),
        category: category.into(),
        input_preview: format!("request {trace}"),
        agent_count: 1,
    }
}

fn traffic() -> Memory {
    let billed = run("t1", "recorded:Billing", "Billing");
    let calls = RunCalls::from([(
        run_key(&billed),
        vec![Call {
            span_id: "s1".into(),
            agent: AGENT.into(),
            duration_ms: 1_200.0,
            charges: vec![Charge {
                identity: LedgerRequest {
                    team_id: "team".into(),
                    start_ms: 10,
                    request_id: "r1".into(),
                },
                spend: Some(0.25),
                column: Column::Deployment {
                    model_id: "dep-vision".into(),
                    model_group: "sonnet".into(),
                    provider: "anthropic".into(),
                },
            }],
            fallback: Column::Unmatched {
                model: "sonnet".into(),
            },
        }],
    )]);
    Memory {
        runs: vec![
            billed,
            run("t2", "recorded:Refunds", "Refunds"),
            run("t3", "cluster:c1", "login help"),
        ],
        calls,
        ..Memory::default()
    }
}

fn token(role: Role) -> String {
    identity::delegated(Identity {
        user_role: role,
        user_id: Some("caller".into()),
        log_team_ids: vec!["team".into()],
        ..Identity::default()
    })
}

async fn serve(database: &Database, store: Memory) -> Server {
    database
        .serve_router(false, |auth| lens_server::profiles::router(auth, store))
        .await
}

async fn call(
    server: &Server,
    method: Method,
    path: &str,
    role: Option<Role>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let request = server
        .client
        .request(method, server.url.join(path).expect("url"));
    let request = match role {
        Some(role) => request.bearer_auth(token(role)),
        None => request,
    };
    let request = match body {
        Some(body) => request.json(&body),
        None => request,
    };
    let response = request.send().await.expect("response");
    let status = response.status();
    (status, response.json().await.unwrap_or(Value::Null))
}

fn window_query(extra: &str) -> String {
    format!("?agent={AGENT}&start_ms=0&end_ms=100{extra}")
}

#[fixture]
fn admin() -> Option<Role> {
    Some(Role::ProxyAdmin)
}

#[rstest]
#[tokio::test]
async fn the_overview_ranks_recorded_and_automatic_profiles_with_their_spend(
    #[future(awt)] database: Database,
    admin: Option<Role>,
) {
    let store = traffic();
    let server = serve(&database, store.clone()).await;

    let (status, body) = call(
        &server,
        Method::GET,
        &format!("/lens/profiles/overview{}", window_query("")),
        admin,
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["profiles"].as_array().map(|profiles| profiles
            .iter()
            .map(|profile| (profile["name"].clone(), profile["source"].clone()))
            .collect::<Vec<_>>()),
        Some(vec![
            (json!("login help"), json!("local")),
            (json!("Billing"), json!("recorded")),
            (json!("Refunds"), json!("recorded")),
        ])
    );
    assert_eq!(body["totals"]["spend_usd"], json!(0.25));
    assert_eq!(
        store
            .queries
            .lock()
            .expect("queries")
            .last()
            .map(|query| (query.agent.clone(), query.limit)),
        Some((AGENT.to_owned(), 5_001))
    );
}

#[rstest]
#[case::span_budget_reached(3, json!({"runs_listed": 3, "runs_resolved": 1, "truncated": false}))]
#[case::run_cap_reached(5_001, json!({"runs_listed": 5_000, "runs_resolved": 1, "truncated": true}))]
#[tokio::test]
async fn only_a_capped_run_list_is_reported_as_truncated(
    #[future(awt)] database: Database,
    admin: Option<Role>,
    #[case] listed: usize,
    #[case] coverage: Value,
) {
    let store = traffic();
    let extra = (store.runs.len()..listed)
        .map(|index| run(&format!("extra-{index}"), "unclassified", "Unclassified"));
    let runs = store.runs.iter().cloned().chain(extra).collect();
    let server = serve(&database, Memory { runs, ..store }).await;

    let (status, body) = call(
        &server,
        Method::GET,
        &format!("/lens/profiles/overview{}", window_query("")),
        admin,
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["coverage"], coverage);
}

#[rstest]
#[case::admin(Role::ProxyAdmin, 2)]
#[case::scoped_user(Role::InternalUser, 1)]
#[tokio::test]
async fn the_matrix_lists_idle_deployments_only_to_every_team_callers(
    #[future(awt)] database: Database,
    #[case] role: Role,
    #[case] columns: usize,
) {
    let server = serve(&database, traffic()).await;

    let (status, body) = call(
        &server,
        Method::GET,
        &format!("/lens/profiles/matrix{}", window_query("")),
        Some(role),
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["models"].as_array().map(Vec::len), Some(columns));
    assert_eq!(body["cells"][0]["column_key"], json!("id:dep-vision"));
}

#[rstest]
#[case::by_id("&profile_id=recorded:Billing")]
#[case::by_legacy_label("&profile_id=Billing")]
#[tokio::test]
async fn the_detail_returns_a_profile_with_its_runs_and_deployments(
    #[future(awt)] database: Database,
    admin: Option<Role>,
    #[case] profile: &str,
) {
    let server = serve(&database, traffic()).await;

    let (status, body) = call(
        &server,
        Method::GET,
        &format!("/lens/profiles/detail{}", window_query(profile)),
        admin,
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["profile"]["profile_id"], json!("recorded:Billing"));
    assert_eq!(body["runs"][0]["trace_id"], json!("t1"));
    assert_eq!(body["models"][0]["column_key"], json!("id:dep-vision"));
}

#[rstest]
#[case::no_profile(
    "/lens/profiles/detail?agent=assistant&start_ms=0&end_ms=100",
    StatusCode::BAD_REQUEST
)]
#[case::unknown_profile(
    "/lens/profiles/detail?agent=assistant&start_ms=0&end_ms=100&profile_id=recorded:Gone",
    StatusCode::NOT_FOUND
)]
#[case::reversed_window(
    "/lens/profiles/overview?agent=assistant&start_ms=200&end_ms=100",
    StatusCode::BAD_REQUEST
)]
#[case::no_agent(
    "/lens/profiles/matrix?agent=%20&start_ms=0&end_ms=100",
    StatusCode::BAD_REQUEST
)]
#[tokio::test]
async fn profile_reads_reject_unusable_requests(
    #[future(awt)] database: Database,
    admin: Option<Role>,
    #[case] path: &str,
    #[case] expected: StatusCode,
) {
    let server = serve(&database, traffic()).await;

    let (status, _) = call(&server, Method::GET, path, admin, None).await;

    assert_eq!(status, expected);
}

#[rstest]
#[case::anonymous_caller(None, false, StatusCode::UNAUTHORIZED)]
#[case::caller_without_trace_access(Some(Role::Customer), false, StatusCode::FORBIDDEN)]
#[case::storage_outage(Some(Role::ProxyAdmin), true, StatusCode::SERVICE_UNAVAILABLE)]
#[tokio::test]
async fn profile_reads_fail_closed(
    #[future(awt)] database: Database,
    #[case] role: Option<Role>,
    #[case] unavailable: bool,
    #[case] expected: StatusCode,
) {
    let store = Memory {
        unavailable,
        ..traffic()
    };
    let server = serve(&database, store).await;
    let token = role.map(|role| {
        identity::delegated(Identity {
            user_role: role,
            token: Some("anonymous-key".into()),
            ..Identity::default()
        })
    });
    let request = server.client.get(
        server
            .url
            .join(&format!("/lens/profiles/overview{}", window_query("")))
            .expect("url"),
    );
    let request = match token {
        Some(token) => request.bearer_auth(token),
        None => request,
    };

    let status = request.send().await.expect("response").status();

    assert_eq!(status, expected);
}
