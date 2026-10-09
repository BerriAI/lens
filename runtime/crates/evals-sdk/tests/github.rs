use lens_evals_sdk::{github::GitHub, model::*};
use rstest::rstest;
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_partial_json, method, path},
};

#[rstest]
#[case::human("human", "POST", "/repos/org/repo/issues/7/comments")]
#[case::bot("github-actions[bot]", "PATCH", "/repos/org/repo/issues/comments/4")]
#[tokio::test]
async fn only_updates_owned_comments(#[case] owner: &str, #[case] verb: &str, #[case] route: &str) {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!([{"id":4,"body":"<!-- lens:demo --> old","user":{"login":owner}}]),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method(verb))
        .and(path(route))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/repos/org/repo/check-runs"))
        .and(body_partial_json(
            json!({"conclusion":"neutral","head_sha":"sha"}),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    let mut run: EvalRun = serde_json::from_str(include_str!(
        "../../../../packages/sdk/tests/fixtures/lens_eval/eval_run_no_baseline.json"
    ))
    .unwrap();
    run.pr = Some(7);
    run.summary.as_mut().unwrap().gate.passed = false;
    GitHub::new(&server.uri(), "token", "org/repo")
        .unwrap()
        .publish(
            &Report {
                run,
                baseline: None,
                trials: vec![],
            },
            "sha",
        )
        .await
        .unwrap();
}
