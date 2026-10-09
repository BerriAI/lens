mod datasets {
    pub mod support;
}

use datasets::support::{ADMIN, Database, database};
use litellm_lens::gateway::{GatewayConfig, Models};
use rstest::rstest;
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[rstest]
#[tokio::test]
async fn signals_validate_against_refreshed_models_without_a_restart(
    #[future(awt)] database: Database,
) {
    let gateway = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/model_group/info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[]})))
        .up_to_n_times(1)
        .mount(&gateway)
        .await;
    let registry = Models::new(
        vec![],
        vec![],
        None,
        Some(GatewayConfig::new(&gateway.uri(), "private-gateway-fixture-key".into()).unwrap()),
    )
    .unwrap();
    let server = database.serve_registry(true, registry.clone()).await;
    let client = reqwest::Client::new();
    let config = json!({"model":"new/evaluator","threshold":0.5,"signals":[{"id":"correct","name":"Correctness","question":"Is the result correct?"}]});
    let missing = client
        .put(format!("{}/lens/signals", server.url))
        .bearer_auth(ADMIN)
        .json(&config)
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), 400);
    Mock::given(method("GET")).and(path("/model_group/info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[
            {"model_group":"new/evaluator","mode":"evaluation"},
            {"model_group":"ordinary-chat","mode":"chat","input_cost_per_token":0.000001,"output_cost_per_token":0.000002,"max_output_tokens":4096}
        ]}))).mount(&gateway).await;
    let refreshed: Value = client
        .post(format!("{}/lens/gateway/refresh", server.url))
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(refreshed["connected"], true);
    assert_eq!(refreshed["evaluation_models"], 1);
    assert_eq!(registry.evaluation().models(), vec!["new/evaluator"]);
    let saved = client
        .put(format!("{}/lens/signals", server.url))
        .bearer_auth(ADMIN)
        .json(&config)
        .send()
        .await
        .unwrap();
    assert_eq!(saved.status(), 200);
    let mut chat = config;
    chat["model"] = json!("ordinary-chat");
    let rejected = client
        .put(format!("{}/lens/signals", server.url))
        .bearer_auth(ADMIN)
        .json(&chat)
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), 400);
    let public: Value = client
        .get(format!("{}/lens/model_group/info", server.url))
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(public["data"].as_array().unwrap().len(), 2);
    assert!(!public.to_string().contains("private-gateway-fixture-key"));
}
