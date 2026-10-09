use lens_evals_sdk::{
    client::Client,
    github::{GitHub, load_report, publish_via_app},
    model::*,
};
use rstest::rstest;
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
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

#[rstest]
#[case::starting("report-start")]
#[case::failed("report-failed")]
fn progress_commands_accept_a_dispatch_pull_request(#[case] command: &str) {
    let parsed = lens_evals_sdk::setup::parse(&[
        "lens".into(),
        command.into(),
        "--name".into(),
        "demo".into(),
        "--pr".into(),
        "7".into(),
    ]);
    assert_eq!(parsed, json!({"command":command,"name":"demo","pr":7}));
}

#[rstest]
#[tokio::test]
async fn running_comment_is_updated_with_real_results_in_place() {
    let server = MockServer::start().await;
    let reads = Arc::new(AtomicUsize::new(0));
    Mock::given(method("GET"))
        .and(path("/repos/org/repo/issues/7/comments"))
        .respond_with(move |_: &wiremock::Request| {
            let comments = if reads.fetch_add(1, Ordering::SeqCst) == 0 {
                json!([])
            } else {
                json!([{"id":42,"body":"<!-- lens:demo --> running","user":{"login":"github-actions[bot]"}}])
            };
            ResponseTemplate::new(200).set_body_json(comments)
        })
        .expect(2).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/repos/org/repo/issues/7/comments"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id":42})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/repos/org/repo/issues/comments/42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":42})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/repos/org/repo/check-runs"))
        .and(body_partial_json(
            json!({"head_sha":"sha","status":"completed","conclusion":"neutral"}),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    let github = GitHub::new(&server.uri(), "token", "org/repo").unwrap();
    github
        .progress(
            "demo",
            &Execution {
                version: "sha".into(),
                branch: "topic".into(),
                pr: Some(7),
                ci_url: "https://github.com/org/repo/actions/runs/99".into(),
                identity: "99:1".into(),
                baseline_run_id: None,
            },
            false,
        )
        .await
        .unwrap();
    let mut run: EvalRun = serde_json::from_str(include_str!(
        "../../../../sdk/tests/fixtures/lens_eval/eval_run_no_baseline.json"
    ))
    .unwrap();
    run.pr = Some(7);
    github
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
    let requests = server.received_requests().await.unwrap();
    let started: serde_json::Value = requests[1].body_json().unwrap();
    let updated: serde_json::Value = requests[3].body_json().unwrap();
    assert!(
        started["body"]
            .as_str()
            .unwrap()
            .contains("I'm running here")
    );
    assert!(
        updated["body"]
            .as_str()
            .unwrap()
            .contains("95% confidence interval")
    );
    assert!(updated["body"].as_str().unwrap().contains("36/36"));
    assert!(
        !updated["body"]
            .as_str()
            .unwrap()
            .contains("I'm running here")
    );
}

#[rstest]
#[tokio::test]
async fn final_report_fetches_candidate_and_selected_baseline_from_lens() {
    let server = MockServer::start().await;
    let baseline: EvalRun = serde_json::from_str(include_str!(
        "../../../../sdk/tests/fixtures/lens_eval/eval_run_no_baseline.json"
    ))
    .unwrap();
    let mut candidate = baseline.clone();
    candidate.id = "candidate".into();
    candidate.summary.as_mut().unwrap().baseline_run_id = Some(baseline.id.clone());
    Mock::given(method("GET"))
        .and(path("/lens/evals/runs/candidate"))
        .and(header("Authorization", "Bearer lens-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&candidate))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/lens/evals/runs/baseline"))
        .and(header("Authorization", "Bearer lens-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&baseline))
        .expect(1)
        .mount(&server)
        .await;
    let report = load_report(
        &Client::new(&server.uri(), "lens-key").unwrap(),
        "candidate",
    )
    .await
    .unwrap();
    assert_eq!(report.run.id, candidate.id);
    assert_eq!(report.baseline.as_ref().unwrap().id, baseline.id);
    assert_eq!(
        report.summary().unwrap().passed,
        candidate.summary.unwrap().passed
    );
}

#[rstest]
#[tokio::test]
async fn failure_replaces_running_comment_without_claiming_an_eval_verdict() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id":42,"body":"<!-- lens:demo --> running","user":{"login":"github-actions[bot]"}}
        ])))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/repos/org/repo/issues/comments/42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/repos/org/repo/check-runs"))
        .and(body_partial_json(
            json!({"status":"completed","conclusion":"failure","head_sha":"sha"}),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    GitHub::new(&server.uri(), "token", "org/repo")
        .unwrap()
        .progress(
            "demo",
            &Execution {
                version: "sha".into(),
                branch: "topic".into(),
                pr: Some(7),
                ci_url: "https://github.com/org/repo/actions/runs/99".into(),
                identity: "99:1".into(),
                baseline_run_id: None,
            },
            true,
        )
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    let body: serde_json::Value = requests[1].body_json().unwrap();
    assert!(
        body["body"]
            .as_str()
            .unwrap()
            .contains("No benchmark verdict is available")
    );
}
