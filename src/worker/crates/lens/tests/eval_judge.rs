use std::collections::BTreeMap;

use chrono::Utc;
use lens_evals::{EvalSpan, Judge, JudgeRequest, SpanStatus};
use lens_server::eval_closer::{ResolvedTrial, RunScoreInput};
use litellm_lens::{Error, config::http_client, eval_judge::GatewayJudge};
use litellm_storage_clickhouse::evals::{StoredCase, StoredRun, StoredTrial};
use litellm_traces_clickhouse::evals::EvalSpan as FullSpan;
use rstest::{fixture, rstest};
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

#[fixture]
fn input() -> RunScoreInput {
    let trial = StoredTrial {
        case_id: "case-a".into(),
        trial: 0,
        result: serde_json::from_value(json!({
            "trace":{"attribute":"trace_id","value":"trace-a"}
        }))
        .unwrap(),
        submitted_at: Utc::now(),
    };
    let span = FullSpan {
        span_id: "selected-span".into(),
        parent_span_id: String::new(),
        name: "agent".into(),
        start_ns: 100,
        end_ns: 200,
        status: litellm_traces::SpanStatus::Ok,
        attributes: BTreeMap::from([("tool.result".into(), "success".into())]),
        input: "Please run the tests".into(),
        output: "The tests passed".into(),
    };
    RunScoreInput {
        run: StoredRun {
            team: "team-a".into(),
            request: serde_json::from_str(include_str!(
                "../../contract/fixtures/lens_eval/create_run.json"
            ))
            .unwrap(),
            run: serde_json::from_str(include_str!(
                "../../contract/fixtures/lens_eval/eval_run_done.json"
            ))
            .unwrap(),
            cases: vec![StoredCase {
                id: "case-a".into(),
                title: "Run the tests".into(),
                critical: false,
                expected: "Tests are run before reporting success".into(),
            }],
            trials: vec![trial.clone()],
            verdicts: BTreeMap::new(),
            created_at: Utc::now(),
            scoring_at: None,
            finished_at: None,
            scoring_lease: None,
        },
        trials: vec![
            ResolvedTrial {
                stored: StoredTrial {
                    trial: 1,
                    ..trial.clone()
                },
                spans: vec![FullSpan {
                    span_id: "unrelated-span".into(),
                    output: "Other trial evidence".into(),
                    ..span.clone()
                }],
            },
            ResolvedTrial {
                stored: trial,
                spans: vec![span],
            },
        ],
        baseline: None,
    }
}

#[fixture]
fn spans() -> Vec<EvalSpan> {
    vec![EvalSpan {
        span_id: "selected-span".into(),
        parent_span_id: String::new(),
        name: "agent".into(),
        start_ns: 100,
        status: SpanStatus::Ok,
        tool_name: None,
        input: "Please run the tests".into(),
        output: "The tests passed".into(),
    }]
}

fn completion(content: &str, finish_reason: &str) -> Value {
    json!({"choices":[{"message":{"content":content},"finish_reason":finish_reason}]})
}

#[rstest]
#[case::explicit_model("chosen-model", "chosen-model")]
#[case::default_model("", "default-model")]
#[tokio::test]
async fn gateway_judge_uses_the_selected_trial_and_attributes_the_call(
    input: RunScoreInput,
    spans: Vec<EvalSpan>,
    #[case] requested_model: &str,
    #[case] expected_model: &str,
    #[values(false, true)] connected: bool,
) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/gateway/chat/completions"))
        .and(header("authorization", "Bearer test-judge-key"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(completion(r#"{"score":0.75}"#, "stop")),
        )
        .expect(1)
        .mount(&server)
        .await;
    let judge = GatewayJudge::new(
        http_client().unwrap(),
        format!("{}/gateway", server.uri()).parse().unwrap(),
        Some("test-judge-key".into()),
        Some("default-model".into()),
    )
    .with_gateway(connected.then(|| {
        lens_inference::GatewayIdentity::new(
            format!("{}/gateway", server.uri()).parse().unwrap(),
            "eval-gateway-signing-secret-with-32-characters",
        )
        .unwrap()
    }));
    let score = judge
        .for_run(&input)
        .score(JudgeRequest {
            case_id: "case-a",
            prompt: "Did the agent run tests?",
            model: requested_model,
            spans: &spans,
        })
        .await
        .unwrap();
    assert_eq!(score, 0.75);
    let requests = server.received_requests().await.unwrap();
    let request: Value = requests[0].body_json().unwrap();
    assert_eq!(request["model"], expected_model);
    assert_eq!(
        requests[0].headers.contains_key("x-lens-internal"),
        connected
    );
    assert_eq!(request["metadata"]["litellm_lens_internal"], true);
    assert_eq!(request["metadata"]["deployment.environment"], "lens-eval");
    assert_eq!(request["metadata"]["lens_team_id"], input.run.team);
    assert_eq!(request["metadata"]["lens_eval_run_id"], input.run.run.id);
    assert_eq!(request["metadata"]["lens_case_id"], "case-a");
    let evidence: Value =
        serde_json::from_str(request["messages"][1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(evidence["rubric"], "Did the agent run tests?");
    assert_eq!(evidence["expected"], input.run.cases[0].expected);
    assert_eq!(evidence["spans"].as_array().unwrap().len(), 1);
    assert_eq!(evidence["spans"][0]["span_id"], "selected-span");
    assert_eq!(evidence["spans"][0]["input"], "Please run the tests");
    assert_eq!(evidence["spans"][0]["output"], "The tests passed");
    assert_eq!(evidence["spans"][0]["attributes"]["tool.result"], "success");
}

#[rstest]
#[case::invalid_json("not a score", "stop")]
#[case::text_score(r#"{"score":"0.75"}"#, "stop")]
#[case::negative(r#"{"score":-0.1}"#, "stop")]
#[case::above_one(r#"{"score":1.1}"#, "stop")]
#[case::unfinished(r#"{"score":0.75}"#, "length")]
#[tokio::test]
async fn invalid_judge_responses_do_not_become_scores(
    input: RunScoreInput,
    spans: Vec<EvalSpan>,
    #[case] content: &str,
    #[case] finish_reason: &str,
) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(completion(content, finish_reason)))
        .expect(1)
        .mount(&server)
        .await;
    let judge = GatewayJudge::new(
        http_client().unwrap(),
        server.uri().parse().unwrap(),
        Some("test-judge-key".into()),
        None,
    );
    let error = judge
        .for_run(&input)
        .score(JudgeRequest {
            case_id: "case-a",
            prompt: "Did the agent run tests?",
            model: "chosen-model",
            spans: &spans,
        })
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<Error>(),
        Some(Error::EvalJudgeResponse)
    ));
}

#[rstest]
#[case::missing_key(None, "chosen-model", "LITELLM_API_KEY")]
#[case::missing_model(Some("test-judge-key"), "", "LENS_EVAL_JUDGE_MODEL")]
#[tokio::test]
async fn missing_judge_configuration_fails_without_a_gateway_call(
    input: RunScoreInput,
    spans: Vec<EvalSpan>,
    #[case] key: Option<&str>,
    #[case] model: &str,
    #[case] missing: &str,
) {
    let server = MockServer::start().await;
    let judge = GatewayJudge::new(
        http_client().unwrap(),
        server.uri().parse().unwrap(),
        key.map(str::to_owned),
        None,
    );
    let error = judge
        .for_run(&input)
        .score(JudgeRequest {
            case_id: "case-a",
            prompt: "Did the agent run tests?",
            model,
            spans: &spans,
        })
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<Error>(),
        Some(Error::Configuration(name)) if *name == missing
    ));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[rstest]
#[case::wrong_case("case-b", "selected-span")]
#[case::wrong_trial("case-a", "missing-span")]
#[tokio::test]
async fn unmatched_evidence_is_not_sent_to_the_gateway(
    input: RunScoreInput,
    mut spans: Vec<EvalSpan>,
    #[case] case_id: &str,
    #[case] span_id: &str,
) {
    let server = MockServer::start().await;
    let judge = GatewayJudge::new(
        http_client().unwrap(),
        server.uri().parse().unwrap(),
        Some("test-judge-key".into()),
        None,
    );
    spans[0].span_id = span_id.into();
    let error = judge
        .for_run(&input)
        .score(JudgeRequest {
            case_id,
            prompt: "Did the agent run tests?",
            model: "chosen-model",
            spans: &spans,
        })
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<Error>(),
        Some(Error::EvalJudgeEvidence)
    ));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[rstest]
#[case::direct_explicit(false, "judge")]
#[case::direct_default(false, "")]
#[case::connected(true, "judge")]
#[tokio::test]
async fn standalone_judge_uses_configured_models_and_only_marks_the_explicit_gateway(
    input: RunScoreInput,
    spans: Vec<EvalSpan>,
    #[case] connected: bool,
    #[case] model: &str,
) {
    use lens_analysis::{AnalysisModels, Catalog, Deployment, Provider, Secret};
    use std::sync::Arc;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .and(header("authorization", "Bearer test-judge-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model":"fixture", "choices":[{"message":{"content":"{\"score\":0.75}"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":100,"completion_tokens":10}
        })))
        .expect(1).mount(&server).await;
    let catalog = Catalog::parse(serde_json::to_vec(&json!({
        "fixture":{"mode":"chat","litellm_provider":"openai","input_cost_per_token":0.000001,"output_cost_per_token":0.000002,"max_input_tokens":10000,"max_output_tokens":400}
    })).unwrap().as_slice(), Default::default()).unwrap();
    let gateway = connected.then(|| {
        lens_inference::GatewayIdentity::new(
            server.uri().parse().unwrap(),
            "eval-gateway-signing-secret-with-32-characters",
        )
        .unwrap()
    });
    let models = AnalysisModels::new(
        Arc::new(catalog),
        vec![Deployment {
            name: "judge".into(),
            model: "fixture".into(),
            provider: Provider::OpenAiCompatible,
            api_base: Some(server.uri().parse().unwrap()),
            api_key: Secret::new("test-judge-key"),
            input_cost_per_token: None,
            output_cost_per_token: None,
            capacity: Default::default(),
            output_limits: Default::default(),
        }],
        Default::default(),
    )
    .unwrap()
    .with_gateway(gateway);
    let judge = GatewayJudge::unconfigured().with_models(Arc::new(models));
    let score = judge
        .for_run(&input)
        .score(JudgeRequest {
            case_id: "case-a",
            prompt: "Did the agent run tests?",
            model,
            spans: &spans,
        })
        .await
        .unwrap();
    assert_eq!(score, 0.75);
    let requests = server.received_requests().await.unwrap();
    let body: Value = requests[0].body_json().unwrap();
    assert_eq!(body["model"], "fixture");
    let serialized = serde_json::to_string(&body["messages"]).unwrap();
    assert!(serialized.contains("Please run the tests"));
    assert!(serialized.contains("Tests are run before reporting success"));
    assert!(!serialized.contains("Other trial evidence"));
    assert_eq!(
        requests[0].headers.contains_key("x-lens-internal"),
        connected
    );
    if connected {
        assert_eq!(body["metadata"]["litellm_lens_internal"], true);
        assert_eq!(body["metadata"]["deployment.environment"], "lens-eval");
        assert_eq!(body["metadata"]["lens_team_id"], input.run.team);
        assert_eq!(body["metadata"]["lens_eval_run_id"], input.run.run.id);
        assert_eq!(body["metadata"]["lens_case_id"], "case-a");
        let token = requests[0]
            .headers
            .get("x-lens-internal")
            .unwrap()
            .to_str()
            .unwrap();
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::HS256);
        validation.set_audience(&["litellm"]);
        validation.set_issuer(&["litellm-lens"]);
        let claims = jsonwebtoken::decode::<Value>(
            token,
            &jsonwebtoken::DecodingKey::from_secret(
                b"eval-gateway-signing-secret-with-32-characters",
            ),
            &validation,
        )
        .unwrap()
        .claims;
        assert_eq!(claims["sub"], "lens-internal");
        assert_eq!(claims["purpose"], "analysis");
    } else {
        assert!(body.get("metadata").is_none());
    }
}
