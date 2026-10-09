use litellm_lens::{State, Storage, config::http_client, router};
use rstest::{fixture, rstest};
use serde_json::{Value, json};
use std::sync::{Arc, atomic::Ordering};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, query_param},
};

const SERVICE_TOKEN: &str = "optional-gateway-service-token-32-chars";

struct Server {
    url: String,
    storage: MockServer,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[fixture]
async fn server() -> Server {
    let storage = MockServer::start().await;
    let config =
        litellm_traces_clickhouse::Config::new("lens".into(), &storage.uri(), 14, 65_536).unwrap();
    let state = Arc::new(State::connected(
        Storage::new(config, http_client().unwrap(), "query-secret".into()),
        SERVICE_TOKEN.into(),
    ));
    state.schema_ready.store(true, Ordering::Release);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = router(state);
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Server { url, storage, task }
}

#[rstest]
#[case::gateway(SERVICE_TOKEN, 200)]
#[case::wrong("incorrect-secret", 401)]
#[case::missing("", 401)]
#[tokio::test]
async fn connected_service_requires_its_separate_credential(
    #[future(awt)] server: Server,
    #[case] token: &str,
    #[case] expected: u16,
) {
    let response = reqwest::Client::new()
        .get(format!("{}/internal/status", server.url))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), expected);
    if expected == 200 {
        assert_eq!(
            response.json::<Value>().await.unwrap()["storage_ready"],
            true
        );
    }
}

#[rstest]
#[tokio::test]
async fn connected_gateway_cannot_replace_lens_owned_ingestion_credentials(
    #[future(awt)] server: Server,
) {
    let response = reqwest::Client::new()
        .post(format!("{}/internal/credentials", server.url))
        .bearer_auth(SERVICE_TOKEN)
        .json(&json!({"issued_at":litellm_lens::auth::unix_seconds(),"keys":[]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    assert!(server.storage.received_requests().await.unwrap().is_empty());
}

#[rstest]
#[tokio::test]
async fn connected_gateway_reads_using_its_authenticated_user_scope(#[future(awt)] server: Server) {
    let result = json!({"data":[{"agent_name":"scoped-agent","runs":"1","failed_runs":"0","last_seen_ms":"1791405060000","frameworks":[]}]});
    Mock::given(method("POST"))
        .and(query_param("param_all_teams", "0"))
        .and(query_param("param_user_id", "caller"))
        .and(query_param("param_team_ids", "['allowed-team']"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&result))
        .expect(1)
        .mount(&server.storage)
        .await;
    let response = reqwest::Client::new().post(format!("{}/internal/read", server.url)).bearer_auth(SERVICE_TOKEN)
        .json(&json!({"operation":"query","name":"trace_agents","parameters":{
            "all_teams":0,"user_id":"caller","team_ids":["allowed-team"],"start_ms":123,"end_ms":456,"limit":100
        }})).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.json::<Value>().await.unwrap(), result);
}
