use litellm_lens::{State, Storage, config::http_client, router};
use rstest::{fixture, rstest};
use serde_json::{Value, json};
use std::sync::{Arc, atomic::Ordering};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_string_contains, method, query_param},
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
    let config = litellm_traces_clickhouse::Config::new("lens".into(), &storage.uri(), 14).unwrap();
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

#[rstest]
#[case::default(None, Some(u32::from(litellm_traces::request::CONVERSATION_PAGE_SIZE_DEFAULT) + 1))]
#[case::paged(Some(2), Some(3))]
#[case::zero(Some(0), None)]
#[case::over_limit(Some(litellm_traces::request::CONVERSATION_PAGE_SIZE_MAX + 1), None)]
#[tokio::test]
async fn connected_conversation_reads_preserve_scope_and_page_bounds(
    #[future(awt)] server: Server,
    #[case] page_size: Option<u16>,
    #[case] expected_limit: Option<u32>,
) {
    if let Some(limit) = expected_limit {
        Mock::given(method("POST"))
            .and(body_string_contains("AS session_id"))
            .and(query_param("param_all_teams", "0"))
            .and(query_param("param_user_id", "caller"))
            .and(query_param("param_team_ids", "['allowed-team']"))
            .and(query_param("param_filter_team_id", "allowed-team"))
            .and(query_param("param_filter_api_key_hash", "selected-key"))
            .and(query_param("param_trace_ref", "selected-ref"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{
                "trace_ref": "selected-ref", "team_id": "allowed-team",
                "api_key_hash": "selected-key", "session_id": "session", "start_ns": 123
            }]})))
            .expect(1)
            .mount(&server.storage)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("WITH conversation_candidates"))
            .and(query_param("param_all_teams", "0"))
            .and(query_param("param_user_id", "caller"))
            .and(query_param("param_team_ids", "['allowed-team']"))
            .and(query_param("param_filter_team_id", "allowed-team"))
            .and(query_param("param_filter_api_key_hash", "selected-key"))
            .and(query_param("param_limit", limit.to_string()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
            .expect(1)
            .mount(&server.storage)
            .await;
    }
    let response = reqwest::Client::new()
        .post(format!("{}/internal/read", server.url))
        .bearer_auth(SERVICE_TOKEN)
        .json(&json!({
            "operation": "conversation",
            "scope": {
                "all_teams": 0, "user_id": "caller", "team_ids": ["allowed-team"],
                "filter_team_id": "allowed-team", "filter_api_key_hash": "selected-key"
            },
            "trace_id": "trace", "trace_ref": "selected-ref", "cursor": null,
            "page_size": page_size
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        if expected_limit.is_some() { 200 } else { 400 }
    );
    if expected_limit.is_some() {
        assert_eq!(
            response.json::<Value>().await.unwrap(),
            json!({"turns": [], "next_cursor": null})
        );
    } else {
        assert!(server.storage.received_requests().await.unwrap().is_empty());
    }
}

fn gateway_call(trace_id: &str, owner: &[(&str, &str)]) -> Value {
    let owner: Vec<Value> = owner
        .iter()
        .map(|(key, value)| json!({"key": key, "value": {"stringValue": value}}))
        .collect();
    json!({
        "resource": {"attributes": [{"key": "service.name", "value": {"stringValue": "litellm"}}]},
        "scopeSpans": [{"scope": {"name": "litellm"}, "spans": [
            {
                "traceId": trace_id,
                "spanId": "b7ad6b7169203331",
                "name": "chat gpt-4o-mini",
                "startTimeUnixNano": "1791405060000000000",
                "endTimeUnixNano": "1791405061000000000",
                "attributes": owner,
            },
            {
                "traceId": trace_id,
                "spanId": "b7ad6b7169203332",
                "parentSpanId": "b7ad6b7169203331",
                "name": "execute_guardrail",
                "startTimeUnixNano": "1791405060000000000",
                "endTimeUnixNano": "1791405061000000000",
            },
        ]}],
    })
}

#[rstest]
#[case::plain(false)]
#[case::gzip(true)]
#[tokio::test]
async fn gateway_traces_are_owned_by_the_virtual_key_each_call_used(
    #[future(awt)] server: Server,
    #[case] gzip: bool,
) {
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server.storage)
        .await;
    let bytes = serde_json::to_vec(&json!({"resourceSpans": [
        gateway_call("0af7651916cd43dd8448eb211c80319a", &[
            ("litellm.api_key.hash", "key-a"),
            ("litellm.team.id", "team-a"),
            ("litellm.metadata.user_api_key_user_id", "user-a"),
            ("litellm.metadata.user_api_key_org_id", "org-a"),
        ]),
        gateway_call("0af7651916cd43dd8448eb211c80319b", &[("litellm.api_key.hash", "key-b")]),
        gateway_call("0af7651916cd43dd8448eb211c80319c", &[]),
    ]}))
    .unwrap();
    let mut request = reqwest::Client::new()
        .post(format!("{}/v1/traces", server.url))
        .bearer_auth(SERVICE_TOKEN)
        .header("content-type", "application/json");
    let body = if gzip {
        request = request.header("content-encoding", "gzip");
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut encoder, &bytes).unwrap();
        encoder.finish().unwrap()
    } else {
        bytes
    };
    let response = request.body(body).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let requests = server.storage.received_requests().await.unwrap();
    let mut decoded = String::new();
    std::io::Read::read_to_string(
        &mut flate2::read::GzDecoder::new(requests[0].body.as_slice()),
        &mut decoded,
    )
    .unwrap();
    let owners: Vec<Value> = decoded
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .map(|row| {
            json!([
                row["TraceId"],
                row["SpanName"],
                row["TeamId"],
                row["ApiKeyHash"],
                row["UserId"],
                row["ResourceAttributes"]["litellm.org_id"]
            ])
        })
        .collect();
    assert_eq!(
        Value::Array(owners),
        json!([
            [
                "0af7651916cd43dd8448eb211c80319a",
                "chat gpt-4o-mini",
                "team-a",
                "key-a",
                "user-a",
                "org-a"
            ],
            [
                "0af7651916cd43dd8448eb211c80319a",
                "execute_guardrail",
                "team-a",
                "key-a",
                "user-a",
                "org-a"
            ],
            [
                "0af7651916cd43dd8448eb211c80319b",
                "chat gpt-4o-mini",
                "",
                "key-b",
                "",
                ""
            ],
            [
                "0af7651916cd43dd8448eb211c80319b",
                "execute_guardrail",
                "",
                "key-b",
                "",
                ""
            ],
            [
                "0af7651916cd43dd8448eb211c80319c",
                "chat gpt-4o-mini",
                "",
                "",
                "",
                ""
            ],
            [
                "0af7651916cd43dd8448eb211c80319c",
                "execute_guardrail",
                "",
                "",
                "",
                ""
            ],
        ])
    );
}

#[rstest]
#[case::unknown_token_before_tracing_keys_load("incorrect-secret", r#"{"resourceSpans":[]}"#, 503)]
#[case::malformed(SERVICE_TOKEN, "{", 400)]
#[tokio::test]
async fn rejected_gateway_traces_never_reach_storage(
    #[future(awt)] server: Server,
    #[case] token: &str,
    #[case] body: &'static str,
    #[case] expected: u16,
) {
    let response = reqwest::Client::new()
        .post(format!("{}/v1/traces", server.url))
        .bearer_auth(token)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), expected);
    assert!(server.storage.received_requests().await.unwrap().is_empty());
}
