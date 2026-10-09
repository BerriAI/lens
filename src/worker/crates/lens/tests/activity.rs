mod activity {
    pub mod support;
}

use activity::support::{ADMIN, Database, SECRET, database, delegated};
use lens_contract::{
    auth::{Identity, Role},
    investigations::Lens,
};
use lens_investigations::LensRepository;
use rstest::rstest;
use serde_json::{Value, json};
use std::path::PathBuf;

const TRACE: &str = "f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0";

fn seed() -> Lens {
    serde_json::from_str(include_str!("../../parity/seeds/investigation.json")).unwrap()
}

#[rstest]
#[case::implicit_offset("")]
#[case::explicit_zero("?offset=0")]
#[tokio::test]
async fn request_evidence_reads_the_original_spend_log_content(
    #[future(awt)] database: Database,
    #[case] query: &str,
) {
    database.investigations.create(&seed()).await.unwrap();
    let at = (chrono::Utc::now() - chrono::TimeDelta::minutes(5))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let storage = &database.state.storage;
    litellm_storage_clickhouse::execute_statement(&storage.client, storage.config.storage().writer(), &format!(
        "INSERT INTO `{}`.spend_logs FORMAT JSONEachRow\n{}",storage.config.storage().database(),
        json!({"request_id":"evidence-request","team_id":"alpha","api_key":"key","start_time":at,"end_time":at,"model":"model","messages":"question","response":"answer"})
    ),std::time::Duration::from_secs(30)).await.unwrap();
    let id = lens_contract::execution::ExecutionId {
        source: "requests".into(),
        team_id: "alpha".into(),
        trace_id: "evidence-request".into(),
        trace_ref: String::new(),
    }
    .encode();
    let server = database.serve(true).await;
    let response = server
        .client
        .get(
            server
                .url
                .join(&format!("/lens/parity-lens/executions/{id}{query}"))
                .unwrap(),
        )
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let content: Value = response.json().await.unwrap();
    assert_eq!(content["parts"][0]["span_id"], "evidence-request");
    assert_eq!(
        content["parts"][0]["content"],
        "Input: question\nOutput: answer\nError: "
    );
    assert_eq!(content["parts"][0]["kind"], "llm");
}

#[rstest]
#[tokio::test]
async fn discovered_agents_come_from_real_trace_metadata(#[future(awt)] database: Database) {
    let at = (chrono::Utc::now() - chrono::TimeDelta::minutes(5))
        .timestamp_nanos_opt()
        .unwrap();
    database
        .trace_with_agent("agent-trace", "alpha", at, "support-agent")
        .await;
    let server = database.serve(true).await;
    let response = server
        .client
        .get(server.url.join("/lens/agents").unwrap())
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!(["support-agent"])
    );
}

#[rstest]
#[tokio::test]
async fn recorded_activity_contracts_replay_against_real_http_and_clickhouse(
    #[future(awt)] database: Database,
) {
    let at = chrono::Utc::now() - chrono::TimeDelta::minutes(5);
    database
        .trace(TRACE, "alpha", at.timestamp_nanos_opt().unwrap())
        .await;
    database.investigations.create(&seed()).await.unwrap();
    let server = database.serve(true).await;
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../parity/fixtures/activity");
    let report = lens_parity::replay_fixtures(
        &server.url,
        &fixtures,
        &lens_parity::Tokens::new(ADMIN, SECRET),
    )
    .await
    .unwrap();
    assert!(report.failures.is_empty(), "{:#?}", report.failures);
    assert_eq!(report.passed, 38);
}

#[rstest]
#[case::availability("GET","/lens/activity/available",json!(null))]
#[case::agents("GET","/lens/agents",json!(null))]
#[case::preview("POST","/lens/preview/sample",json!({"selection":{}}))]
#[case::evidence("GET","/lens/parity-lens/executions/invalid",json!(null))]
#[case::findings("POST","/lens/traces/findings",json!({"traces":[{"trace_id":"trace"}]}))]
#[tokio::test]
async fn activity_requires_administrator_read_access(
    #[future(awt)] database: Database,
    #[case] method: &str,
    #[case] path: &str,
    #[case] body: Value,
) {
    let server = database.serve(true).await;
    let token = delegated(Identity {
        user_role: Role::InternalUser,
        user_id: Some("caller".into()),
        team_id: Some("alpha".into()),
        ..Identity::default()
    });
    let response = server
        .client
        .request(method.parse().unwrap(), server.url.join(path).unwrap())
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"detail":"Lens requires proxy administrator access"})
    );
}

#[rstest]
#[case::availability("/lens/activity/available",json!({"traces":false,"requests":false}))]
#[case::agents("/lens/agents",json!([]))]
#[tokio::test]
async fn unconfigured_activity_keeps_its_empty_discovery_responses(
    #[future(awt)] database: Database,
    #[case] path: &str,
    #[case] expected: Value,
) {
    let server = database.serve(false).await;
    let response = server
        .client
        .get(server.url.join(path).unwrap())
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.json::<Value>().await.unwrap(), expected);
}

#[rstest]
#[tokio::test]
async fn preview_requires_a_configured_source(#[future(awt)] database: Database) {
    let server = database.serve(false).await;
    let response = server
        .client
        .post(server.url.join("/lens/preview/sample").unwrap())
        .bearer_auth(ADMIN)
        .json(&json!({"selection":{}}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 501);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"detail":"Agent tracing is not enabled. Configure the Lens service and LITELLM_LENS_URL."})
    );
}

#[rstest]
#[tokio::test]
async fn preview_reads_recent_completed_traces_and_applies_team_selection(
    #[future(awt)] database: Database,
) {
    let at = (chrono::Utc::now() - chrono::TimeDelta::minutes(5))
        .timestamp_nanos_opt()
        .unwrap();
    database.trace(TRACE, "alpha", at).await;
    database.trace("foreign", "beta", at).await;
    let server = database.serve(true).await;
    let token = delegated(Identity {
        user_role: Role::ProxyAdminViewer,
        user_id: Some("viewer".into()),
        ..Identity::default()
    });
    let response = server
        .client
        .post(server.url.join("/lens/preview/sample").unwrap())
        .bearer_auth(token)
        .json(&json!({"selection":{"team_id":"alpha"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["eligible"], 1);
    assert_eq!(body["selected"], 1);
    assert_eq!(body["executions"].as_array().unwrap().len(), 1);
    assert_eq!(body["executions"][0]["trace_id"], TRACE);
    assert_eq!(body["executions"][0]["team_id"], "alpha");
    assert_eq!(body["next_offset"], Value::Null);
}

#[rstest]
#[case::models("/models")]
#[case::openai_models("/v1/models")]
#[case::details("/model_group/info")]
#[case::lens_models("/lens/models")]
#[case::lens_details("/lens/model_group/info")]
#[tokio::test]
async fn model_dropdowns_use_only_configured_models(
    #[future(awt)] database: Database,
    #[case] path: &str,
) {
    let info = lens_contract::activity::AnalysisModelInfo {
        model_group: "fixture-analysis".into(),
        providers: vec!["openai_compatible".into()],
        mode: "chat".into(),
        supported_openai_params: None,
    };
    let server = database.serve_models(true, vec![info.clone()]).await;
    let token = delegated(Identity {
        user_role: Role::ProxyAdminViewer,
        user_id: Some("viewer".into()),
        ..Identity::default()
    });
    let response = server
        .client
        .get(server.url.join(path).unwrap())
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let expected = if path.ends_with("/model_group/info") {
        json!({"data":[info]})
    } else {
        json!({"data":[{"id":"fixture-analysis"}]})
    };
    assert_eq!(response.json::<Value>().await.unwrap(), expected);
}

#[rstest]
#[case::anonymous(None, 401)]
#[case::team(Some(Role::InternalUser), 403)]
#[case::administrator(Some(Role::ProxyAdmin), 200)]
#[tokio::test]
async fn an_unconfigured_model_catalog_is_empty_and_authorized(
    #[future(awt)] database: Database,
    #[case] role: Option<Role>,
    #[case] status: u16,
) {
    let server = database.serve(true).await;
    let request = server.client.get(server.url.join("/models").unwrap());
    let request = match role {
        Some(role) => request.bearer_auth(delegated(Identity {
            user_role: role,
            user_id: Some("caller".into()),
            ..Identity::default()
        })),
        None => request,
    };
    let response = request.send().await.unwrap();
    assert_eq!(response.status(), status);
    if status == 200 {
        assert_eq!(response.json::<Value>().await.unwrap(), json!({"data":[]}));
    }
}

#[rstest]
#[case::anonymous_read(None, "GET", "/lens/gateway", 401)]
#[case::anonymous_refresh(None, "POST", "/lens/gateway/refresh", 401)]
#[case::team_read(Some(Role::InternalUser), "GET", "/lens/gateway", 403)]
#[case::viewer_read(Some(Role::ProxyAdminViewer), "GET", "/lens/gateway", 200)]
#[case::viewer_refresh(Some(Role::ProxyAdminViewer), "POST", "/lens/gateway/refresh", 403)]
#[case::admin_refresh(Some(Role::ProxyAdmin), "POST", "/lens/gateway/refresh", 200)]
#[tokio::test]
async fn gateway_discovery_is_admin_only_and_status_is_viewer_readable(
    #[future(awt)] database: Database,
    #[case] role: Option<Role>,
    #[case] method: &str,
    #[case] path: &str,
    #[case] expected: u16,
) {
    let server = database.serve(true).await;
    let request = server
        .client
        .request(method.parse().unwrap(), server.url.join(path).unwrap());
    let request = match role {
        Some(role) => request.bearer_auth(delegated(Identity {
            user_role: role,
            user_id: Some("caller".into()),
            ..Identity::default()
        })),
        None => request,
    };
    let response = request.send().await.unwrap();
    assert_eq!(response.status(), expected);
    if expected == 200 {
        assert_eq!(response.json::<Value>().await.unwrap()["configured"], false);
    }
}

#[rstest]
#[case::agents("/lens/agents")]
#[case::availability("/lens/activity/available")]
#[case::preview("/lens/preview/sample")]
#[case::findings("/lens/traces/findings")]
#[case::execution("/lens/id/executions/id")]
#[tokio::test]
async fn activity_trailing_slashes_preserve_the_public_redirect(
    #[future(awt)] database: Database,
    #[case] path: &str,
) {
    let server = database.serve(true).await;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let response = client
        .get(server.url.join(&format!("{path}/?x=1")).unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 307);
    assert_eq!(
        response.headers()["location"],
        server.url.join(&format!("{path}?x=1")).unwrap().as_str()
    );
}
