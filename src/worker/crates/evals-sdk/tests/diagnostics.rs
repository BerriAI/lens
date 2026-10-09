mod support;
use lens_evals_sdk::{client::Client, doctor, model::*, setup::Settings};
use rstest::rstest;
use serde_json::json;
use support::{Server, server, spec};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, path_regex},
};

#[rstest]
#[tokio::test]
async fn doctor_checks_without_creating_runs(#[future] server: Server, spec: EvalSpec) {
    let server = server.await;
    let checks = doctor::diagnose(
        &server.client,
        &Settings {
            project: "demo".into(),
            ..Settings::default()
        },
        &[spec],
    )
    .await;
    assert_eq!(checks.iter().filter(|check| check.ok).count(), 3);
    assert!(checks[1].detail.contains("36 included cases, 108 trials"));
    let runs: Vec<EvalRun> = Client::decode(
        server
            .client
            .request(reqwest::Method::GET, "/lens/evals/runs", None, None, false)
            .await
            .unwrap(),
    )
    .await
    .unwrap();
    assert!(runs.is_empty());
}

#[rstest]
#[case::missing_api(404, "request_failed", "eval API")]
#[case::scope(403, "request_failed", "Lens routes")]
#[tokio::test]
async fn doctor_distinguishes_missing_api_and_scope(
    spec: EvalSpec,
    #[case] status: u16,
    #[case] code: &str,
    #[case] hint: &str,
) {
    let server = MockServer::start().await;
    Mock::given(path("/lens/datasets/resolve"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id":"demo","name":"demo","revision":1})),
        )
        .mount(&server)
        .await;
    Mock::given(path("/lens/datasets/demo/revisions/1/cases"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(lens_evals_sdk::devserver::sample_cases(1)),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex("/lens/evals/runs/.*"))
        .respond_with(ResponseTemplate::new(status).set_body_json(json!({"code":code})))
        .expect(1)
        .mount(&server)
        .await;
    let checks = doctor::diagnose(
        &Client::new(&server.uri(), "key").unwrap(),
        &Settings {
            project: "demo".into(),
            ..Settings::default()
        },
        &[spec],
    )
    .await;
    assert!(checks[1].ok);
    assert!(!checks[2].ok);
    assert!(checks[2].detail.contains(hint));
    assert_eq!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.method.as_str() != "GET")
            .count(),
        0
    );
}
