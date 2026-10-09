use std::{collections::BTreeMap, sync::Arc, time::Duration};

use lens_contract::signals::SignalStep;
use lens_decisions::{Deployment, EvaluationModels, Provider, Secret};
use lens_signals::{DecisionRequest, Question, SignalState};
use litellm_model_catalog::Catalog;
use rstest::{fixture, rstest};
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

#[fixture]
fn request() -> DecisionRequest {
    DecisionRequest {
        model: "system-one".into(),
        state: SignalState {
            task: "Classify the trace",
            steps: vec![SignalStep {
                kind: "user".into(),
                name: "message".into(),
                content: "I asked three times and this still does not work".into(),
            }],
        },
        questions: BTreeMap::from([
            (
                "frustration".into(),
                Question::Noul {
                    instructions: "Does the user express frustration?".into(),
                },
            ),
            (
                "repeated".into(),
                Question::Noul {
                    instructions: "Did the user repeat a request?".into(),
                },
            ),
        ]),
        timeout: Duration::from_secs(2),
        tags: vec!["litellm-lens-signals"],
    }
}

fn client(server: &MockServer) -> EvaluationModels {
    EvaluationModels::new(
        Arc::new(
            Catalog::parse(
                br#"{"evaluation-fixture":{"mode":"evaluation","litellm_provider":"typesafe"}}"#,
                Default::default(),
            )
            .unwrap(),
        ),
        vec![Deployment {
            name: "system-one".into(),
            model: "gateway/model-alias".into(),
            provider: Provider::DecisionsCompatible,
            api_base: Some(format!("{}/gateway/v1", server.uri()).parse().unwrap()),
            api_key: Some(Secret::new("private-fixture-key")),
        }],
        Default::default(),
    )
    .unwrap()
}

#[rstest]
#[tokio::test]
async fn gateway_uses_named_predicates_and_restores_signal_ids(request: DecisionRequest) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/gateway/v1/decisions"))
        .and(header("authorization", "Bearer private-fixture-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model":"provider-model",
            "answers":[
                {"type":"predicate","name":"repeated","probability":0.81},
                {"type":"predicate","name":"frustration","probability":0.94}
            ],
            "usage":{"input_tokens":50,"output_tokens":4}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let result = client(&server).evaluate(&request).await.unwrap();
    assert_eq!(
        result["answers"],
        json!({
            "frustration":{"type":"noul","noul":0.94},
            "repeated":{"type":"noul","noul":0.81}
        })
    );
    assert_eq!(result["model"], "provider-model");
    assert_eq!(result["usage"]["input_tokens"], 50);
    let received = server.received_requests().await.unwrap();
    let body: Value = received[0].body_json().unwrap();
    assert_eq!(body["model"], "gateway/model-alias");
    let input: Value = serde_json::from_str(body["input"].as_str().unwrap()).unwrap();
    assert_eq!(input, serde_json::to_value(&request.state).unwrap());
    assert_eq!(
        body["questions"],
        json!([
            {"type":"predicate","name":"frustration","instructions":"Does the user express frustration?"},
            {"type":"predicate","name":"repeated","instructions":"Did the user repeat a request?"}
        ])
    );
    assert_eq!(body["metadata"]["tags"], json!(["litellm-lens-signals"]));
    assert!(body.get("state").is_none());
}

#[rstest]
#[case::duplicate(json!([
    {"type":"predicate","name":"frustration","probability":0.2},
    {"type":"predicate","name":"frustration","probability":0.9}
]))]
#[case::unrequested(json!([{"type":"predicate","name":"other","probability":0.9}]))]
#[case::negative(json!([{"type":"predicate","name":"frustration","probability":-0.1}]))]
#[case::above_one(json!([{"type":"predicate","name":"frustration","probability":1.1}]))]
#[case::missing_name(json!([{"type":"predicate","probability":0.5}]))]
#[case::missing_probability(json!([{"type":"predicate","name":"frustration"}]))]
#[case::wrong_type(json!([{"type":"choice","name":"frustration","choice":"yes"}]))]
#[case::native_map(json!({"frustration":{"type":"noul","noul":0.5}}))]
#[tokio::test]
async fn malformed_gateway_answers_cannot_become_signal_scores(
    request: DecisionRequest,
    #[case] answers: Value,
) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"answers":answers})))
        .mount(&server)
        .await;
    assert!(client(&server).evaluate(&request).await.is_err());
}

#[rstest]
#[tokio::test]
async fn refusal_leaves_the_signal_unanswered_instead_of_scoring_zero(request: DecisionRequest) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "answers":[
                {"type":"refusal","name":"frustration"},
                {"type":"predicate","name":"repeated","probability":0.6}
            ]
        })))
        .mount(&server)
        .await;
    let result = client(&server).evaluate(&request).await.unwrap();
    assert_eq!(
        result["answers"],
        json!({"repeated":{"type":"noul","noul":0.6}})
    );
}

#[rstest]
#[tokio::test]
async fn gateway_selects_evidence_and_scores_in_one_request(mut request: DecisionRequest) {
    request.questions.insert(
        "evidence_frustration".into(),
        Question::Choice {
            instructions: "Which passage shows the user's frustration?".into(),
            criteria: BTreeMap::from([
                ("passage_0".into(), None),
                ("none".into(), Some("No supporting passage".into())),
            ]),
        },
    );
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/gateway/v1/decisions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"answers":[
            {"type":"predicate","name":"frustration","probability":0.94},
            {"type":"predicate","name":"repeated","probability":0.81},
            {"type":"choice","name":"evidence_frustration","choice":"passage_0","confidence":0.9,
             "probabilities":[{"value":"passage_0","probability":0.95},{"value":"none","probability":0.05}]}
        ]})))
        .expect(1)
        .mount(&server)
        .await;
    let result = client(&server).evaluate(&request).await.unwrap();
    assert_eq!(
        result["answers"]["frustration"],
        json!({"type":"noul","noul":0.94})
    );
    assert_eq!(
        result["answers"]["evidence_frustration"],
        json!({"type":"choice","choice":"passage_0","confidence":0.9})
    );
    let received = server.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    let body: Value = received[0].body_json().unwrap();
    assert_eq!(
        body["questions"][0],
        json!({
            "type":"choice","name":"evidence_frustration",
            "instructions":"Which passage shows the user's frustration?",
            "choices":[{"value":"none","description":"No supporting passage"},{"value":"passage_0"}]
        })
    );
}

#[rstest]
#[case::refusal(json!({"type":"refusal","name":"evidence_frustration"}))]
#[case::unknown_passage(json!({"type":"choice","name":"evidence_frustration","choice":"made_up","confidence":0.9}))]
#[case::bad_confidence(json!({"type":"choice","name":"evidence_frustration","choice":"passage_0","confidence":2}))]
#[case::wrong_shape(json!({"type":"choice","name":"evidence_frustration","choice":42,"confidence":"certain"}))]
#[case::missing_fields(json!({"type":"choice","name":"evidence_frustration"}))]
#[case::unknown_type(json!({"type":"unknown","name":"evidence_frustration"}))]
#[case::missing_type(json!({"name":"evidence_frustration"}))]
#[case::malformed_predicate(json!({"type":"predicate","name":"evidence_frustration"}))]
#[tokio::test]
async fn unusable_evidence_keeps_valid_signal_scores(
    mut request: DecisionRequest,
    #[case] evidence: Value,
) {
    request.questions.insert(
        "evidence_frustration".into(),
        Question::Choice {
            instructions: "Which passage supports this signal?".into(),
            criteria: BTreeMap::from([("passage_0".into(), None), ("none".into(), None)]),
        },
    );
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"answers":[
            {"type":"predicate","name":"frustration","probability":0.94}, evidence
        ]})))
        .expect(1)
        .mount(&server)
        .await;
    let result = client(&server).evaluate(&request).await.unwrap();
    assert_eq!(
        result["answers"],
        json!({"frustration":{"type":"noul","noul":0.94}})
    );
}
