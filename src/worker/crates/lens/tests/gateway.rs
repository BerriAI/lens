use std::{collections::BTreeMap, time::Duration};

use lens_contract::worker::{ModelRequest, ModelRequestPurpose};
use lens_server::models::ModelCatalog;
use lens_signals::{DecisionRequest, Question, SignalState};
use litellm_lens::gateway::{GatewayConfig, Models};
use rstest::{fixture, rstest};
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

const KEY: &str = "gateway-discovery-test-secret";

#[fixture]
fn decision() -> DecisionRequest {
    DecisionRequest {
        model: "team/system-one".into(),
        state: SignalState {
            task: "classify",
            steps: vec![],
        },
        questions: BTreeMap::from([(
            "quality".into(),
            Question::Noul {
                instructions: "Was the task completed?".into(),
            },
        )]),
        timeout: Duration::from_secs(2),
        tags: vec!["litellm-lens-signals"],
    }
}

fn models(server: &MockServer) -> Models {
    Models::new(
        vec![],
        vec![],
        None,
        Some(GatewayConfig::new(&format!("{}/gateway/v1", server.uri()), KEY.into()).unwrap()),
    )
    .unwrap()
}

async fn catalog(server: &MockServer, body: Value) {
    Mock::given(method("GET"))
        .and(path("/gateway/model_group/info"))
        .and(header("authorization", format!("Bearer {KEY}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

#[rstest]
#[tokio::test]
async fn discovered_models_are_the_models_used_for_actual_inference(decision: DecisionRequest) {
    let server = MockServer::start().await;
    catalog(&server, json!({"data":[
        {"model_group":"team/system-one","mode":"evaluation"},
        {"model_group":"team/analysis","mode":"chat","input_cost_per_token":0.000001,"output_cost_per_token":0.000002,"max_input_tokens":128000.0,"max_output_tokens":128000.0},
        {"model_group":"embeddings","mode":"embedding"},
        {"model_group":"unknown-mode","mode":null}
    ]})).await;
    let decisions = json!({"answers":[{"type":"predicate","name":"quality","probability":0.9}]});
    Mock::given(method("POST"))
        .and(path("/gateway/v1/decisions"))
        .and(header("authorization", format!("Bearer {KEY}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(&decisions))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path("/gateway/v1/chat/completions")).and(header("authorization", format!("Bearer {KEY}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"completion", "object":"chat.completion", "created":1,"model":"team/analysis","choices":[{"index":0,"message":{"role":"assistant","content":"Result"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":2,"total_tokens":12}})))
        .expect(1).mount(&server).await;
    let models = models(&server);
    let status = models.refresh().await;
    assert!(status.connected, "{:?}", status.error);
    assert_eq!(status.evaluation_models, 1);
    assert_eq!(status.analysis_models, 1);
    assert!(status.last_refreshed.is_some());
    assert_eq!(models.model_groups().len(), 2);
    assert_eq!(
        models.evaluation().evaluate(&decision).await.unwrap(),
        json!({"answers":{"quality":{"type":"noul","noul":0.9}}})
    );
    let analysis = models.analysis();
    let prepared = analysis
        .prepare(
            "team/analysis",
            &ModelRequest {
                messages: vec![],
                prompt: "Check the evidence".try_into().unwrap(),
                purpose: ModelRequestPurpose::Extract,
            },
        )
        .await
        .unwrap();
    assert!(prepared.estimate > 0.0);
    assert!(
        prepared.estimate < 0.01,
        "reserve the requested output, not the full model capacity"
    );
    assert_eq!(
        analysis.complete(&prepared).await.unwrap().result.content,
        "Result"
    );
    let requests = server.received_requests().await.unwrap();
    let decision_body: Value = requests[1].body_json().unwrap();
    assert_eq!(decision_body["model"], "team/system-one");
    assert_eq!(
        decision_body["input"],
        serde_json::to_string(&decision.state).unwrap()
    );
    assert_eq!(
        decision_body["questions"],
        json!([{"type":"predicate","name":"quality","instructions":"Was the task completed?"}])
    );
    assert_eq!(
        decision_body["metadata"]["tags"],
        json!(["litellm-lens-signals"])
    );
    let analysis_body: Value = requests[2].body_json().unwrap();
    assert_eq!(analysis_body["model"], "team/analysis");
    assert_eq!(analysis_body["max_tokens"], 4096);
}

#[rstest]
#[tokio::test]
async fn model_catalogs_larger_than_four_mib_are_discovered() {
    let server = MockServer::start().await;
    catalog(
        &server,
        json!({"data":[{"model_group":"team/system-one","mode":"evaluation","description":"x".repeat(4 * 1024 * 1024)}]}),
    )
    .await;
    let models = models(&server);
    let status = models.refresh().await;
    assert!(status.connected, "{:?}", status.error);
    assert_eq!(models.evaluation().models(), vec!["team/system-one"]);
}

#[rstest]
#[case::unauthorized(401)]
#[case::unavailable(503)]
#[tokio::test]
async fn failed_refresh_preserves_last_good_executable_models(#[case] status: u16) {
    let server = MockServer::start().await;
    catalog(
        &server,
        json!({"data":[{"model_group":"team/system-one","mode":"evaluation"}]}),
    )
    .await;
    let models = models(&server);
    let success = models.refresh().await;
    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(status).set_body_string(KEY))
        .mount(&server)
        .await;
    let failed = models.refresh().await;
    assert!(!failed.connected);
    assert_eq!(failed.last_refreshed, success.last_refreshed);
    assert_eq!(models.evaluation().models(), vec!["team/system-one"]);
    assert!(
        failed
            .error
            .as_deref()
            .unwrap()
            .contains(&status.to_string())
    );
    assert!(!serde_json::to_string(&failed).unwrap().contains(KEY));
}

#[rstest]
#[tokio::test]
async fn refresh_replaces_removed_aliases_without_interrupting_in_flight_snapshots() {
    let server = MockServer::start().await;
    catalog(
        &server,
        json!({"data":[{"model_group":"old","mode":"evaluation"}]}),
    )
    .await;
    let models = models(&server);
    models.refresh().await;
    let running = models.evaluation();
    server.reset().await;
    catalog(
        &server,
        json!({"data":[{"model_group":"new","mode":"evaluation"}]}),
    )
    .await;
    models.refresh().await;
    assert_eq!(running.models(), vec!["old"]);
    assert_eq!(models.evaluation().models(), vec!["new"]);
}

#[rstest]
#[case::missing_price(json!({"max_output_tokens":4096}))]
#[case::negative_price(json!({"input_cost_per_token":-1,"output_cost_per_token":0.1,"max_output_tokens":4096}))]
#[case::missing_limit(json!({"input_cost_per_token":0.1,"output_cost_per_token":0.1}))]
#[case::fractional_limit(json!({"input_cost_per_token":0.1,"output_cost_per_token":0.1,"max_output_tokens":1.5}))]
#[tokio::test]
async fn invalid_chat_metadata_does_not_prevent_system_one_discovery(#[case] mut details: Value) {
    let server = MockServer::start().await;
    details["model_group"] = json!("unknown-chat");
    details["mode"] = json!("chat");
    catalog(
        &server,
        json!({"data":[details,{"model_group":"valid-evaluation","mode":"evaluation"}]}),
    )
    .await;
    let models = models(&server);
    let status = models.refresh().await;
    assert!(status.connected);
    assert_eq!(status.analysis_models, 0);
    assert_eq!(status.evaluation_models, 1);
    assert!(
        status
            .error
            .as_deref()
            .unwrap()
            .starts_with("1 gateway models")
    );
    assert!(models.analysis().models().is_empty());
}

#[rstest]
#[tokio::test]
async fn explicit_static_alias_takes_precedence_over_a_discovered_duplicate() {
    let server = MockServer::start().await;
    catalog(&server, json!({"data":[{"model_group":"reserved","mode":"evaluation"},{"model_group":"discovered","mode":"evaluation"}]})).await;
    let models = Models::new(
        vec![],
        vec![lens_decisions::Deployment {
            name: "reserved".into(),
            model: "explicit-upstream".into(),
            provider: lens_decisions::Provider::DecisionsCompatible,
            api_base: Some(server.uri().parse().unwrap()),
            api_key: Some(lens_decisions::Secret::new("static-key")),
        }],
        None,
        Some(GatewayConfig::new(&format!("{}/gateway", server.uri()), KEY.into()).unwrap()),
    )
    .unwrap();
    let status = models.refresh().await;
    assert_eq!(status.evaluation_models, 1);
    assert_eq!(models.evaluation().models(), vec!["discovered", "reserved"]);
}

#[rstest]
#[tokio::test]
async fn redirects_do_not_forward_gateway_credentials() {
    let origin = MockServer::start().await;
    let other = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("Location", format!("{}/stolen", other.uri())),
        )
        .mount(&origin)
        .await;
    let status = models(&origin).refresh().await;
    assert!(!status.connected);
    assert!(other.received_requests().await.unwrap().is_empty());
}

#[rstest]
#[case::userinfo("https://user:password@example.test")]
#[case::query("https://example.test?api_key=secret")]
#[case::fragment("https://example.test#secret")]
#[case::scheme("file:///etc/passwd")]
fn invalid_base_urls_are_rejected_without_echoing_them(#[case] base: &str) {
    let error = GatewayConfig::new(base, KEY.into()).err().unwrap();
    assert!(!error.to_string().contains(base));
    assert!(!error.to_string().contains(KEY));
}
