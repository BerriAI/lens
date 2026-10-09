use lens_evals_sdk::{
    client::Client,
    github::{GitHub, publish_via_app},
    model::*,
};
use rstest::rstest;
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, body_partial_json, header, method, path},
};

#[rstest]
#[tokio::test]
async fn app_reporting_sends_only_run_ids_to_authenticated_lens() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/lens/github/report"))
        .and(header("Authorization", "Bearer lens-key"))
        .and(header("X-Lens-Contract", "1"))
        .and(body_json(json!({"run_ids": ["stored-run"]})))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"reports": [{"run_id":"stored-run"}]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    publish_via_app(
        &Client::new(&server.uri(), "lens-key").unwrap(),
        &["stored-run"],
    )
    .await
    .unwrap();
}

#[rstest]
#[tokio::test]
async fn app_reporting_rejects_confirmation_for_a_different_run() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/lens/github/report"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"reports": [{"run_id":"different"}]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let result = publish_via_app(
        &Client::new(&server.uri(), "lens-key").unwrap(),
        &["stored-run"],
    )
    .await;
    assert!(matches!(
        result,
        Err(lens_evals_sdk::Error::Infrastructure(_))
    ));
}

#[rstest]
#[tokio::test]
async fn app_reporting_propagates_server_authorization_failure() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/lens/github/report"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({"code": "forbidden"})))
        .expect(1)
        .mount(&server)
        .await;
    let result = publish_via_app(
        &Client::new(&server.uri(), "lens-key").unwrap(),
        &["stored-run"],
    )
    .await;
    assert!(matches!(
        result,
        Err(lens_evals_sdk::Error::Api { status: 403, .. })
    ));
}

#[rstest]
#[case::actions(vec!["lens", "report", "report.json"], false)]
#[case::app(vec!["lens", "report", "report.json", "--via-app"], true)]
fn reporting_mode(#[case] arguments: Vec<&str>, #[case] via_app: bool) {
    let parsed =
        lens_evals_sdk::setup::parse(&arguments.into_iter().map(str::to_owned).collect::<Vec<_>>());
    assert_eq!(
        parsed,
        json!({"command":"report", "file":"report.json", "via_app": via_app})
    );
}

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
            json!({"conclusion":"failure","head_sha":"sha"}),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    let mut run: EvalRun = serde_json::from_str(include_str!(
        "../../../../sdk/tests/fixtures/lens_eval/eval_run_no_baseline.json"
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
