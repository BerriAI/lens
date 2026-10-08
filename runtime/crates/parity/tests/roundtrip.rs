use std::process::Command;

use serde_json::json;
use url::Url;
use wiremock::matchers::{body_json, header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use lens_parity::{
    Capture, CaptureSource, RequestFixture, Scenario, Tokens, record_scenarios, replay_fixtures,
};

fn scenario_steps() -> [Scenario; 3] {
    [
        Scenario {
            directory: "roundtrip".to_owned(),
            name: "00_create".to_owned(),
            request: RequestFixture {
                method: "POST".to_owned(),
                path: "/create".to_owned(),
                headers: Default::default(),
                body: json!({"token": "{{admin_token}}"}),
            },
            captures: vec![Capture {
                binding: "dataset_id".to_owned(),
                source: CaptureSource::JsonPointer("/id".to_owned()),
            }],
        },
        Scenario {
            directory: "roundtrip".to_owned(),
            name: "01_read".to_owned(),
            request: RequestFixture {
                method: "GET".to_owned(),
                path: "/datasets/{{dataset_id}}".to_owned(),
                headers: Default::default(),
                body: serde_json::Value::Null,
            },
            captures: vec![],
        },
        Scenario {
            directory: "roundtrip".to_owned(),
            name: "02_auth".to_owned(),
            request: RequestFixture {
                method: "GET".to_owned(),
                path: "/auth".to_owned(),
                headers: [(
                    "authorization".to_owned(),
                    "Bearer {{jwt.valid}}".to_owned(),
                )]
                .into(),
                body: serde_json::Value::Null,
            },
            captures: vec![],
        },
    ]
}

async fn mount_responses(server: &MockServer, value: &str) {
    Mock::given(method("POST"))
        .and(path("/create"))
        .and(body_json(json!({"token": "parity-admin-token"})))
        .respond_with(
            ResponseTemplate::new(201)
                .insert_header("set-cookie", "single=one; Path=/")
                .set_body_json(json!({"id": "id-1"})),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/datasets/id-1"))
        .respond_with(
            ResponseTemplate::new(200)
                .append_header("set-cookie", "first=one; Path=/")
                .append_header("set-cookie", "second=two; Path=/")
                .set_body_json(json!({"value": value})),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/auth"))
        .and(header_exists("authorization"))
        .respond_with(ResponseTemplate::new(204))
        .mount(server)
        .await;
}

#[rstest::rstest]
#[case::records_then_replays(())]
#[tokio::test]
async fn records_replays_and_reports_mutated_responses(#[case] _scenario: ()) {
    let server = MockServer::start().await;
    mount_responses(&server, "stable").await;
    let fixtures = tempfile::tempdir().unwrap();
    let base_url = server.uri();
    let parsed_base_url = Url::parse(&base_url).unwrap();
    let tokens = Tokens::new("parity-admin-token", "parity-gateway-secret");

    let recorded = record_scenarios(
        &parsed_base_url,
        fixtures.path(),
        &scenario_steps(),
        &tokens,
    )
    .await
    .unwrap();
    let recorded_fixture: lens_parity::Fixture = serde_json::from_slice(
        &tokio::fs::read(fixtures.path().join("roundtrip/01_read.json"))
            .await
            .unwrap(),
    )
    .unwrap();
    let create_fixture: lens_parity::Fixture = serde_json::from_slice(
        &tokio::fs::read(fixtures.path().join("roundtrip/00_create.json"))
            .await
            .unwrap(),
    )
    .unwrap();
    let auth_fixture: lens_parity::Fixture = serde_json::from_slice(
        &tokio::fs::read(fixtures.path().join("roundtrip/02_auth.json"))
            .await
            .unwrap(),
    )
    .unwrap();
    let passed = replay_fixtures(&parsed_base_url, fixtures.path(), &tokens)
        .await
        .unwrap();
    let help = Command::new(env!("CARGO_BIN_EXE_lens-parity"))
        .arg("--help")
        .output()
        .unwrap();
    let cli_pass = Command::new(env!("CARGO_BIN_EXE_lens-parity"))
        .args(["replay", "--base-url"])
        .arg(&base_url)
        .arg("--fixtures")
        .arg(fixtures.path())
        .output()
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    let dataset_request = requests
        .iter()
        .find(|request| request.url.path() == "/datasets/id-1")
        .unwrap();

    assert_eq!(recorded, 3);
    assert_eq!(
        create_fixture.request.body,
        json!({"token": "{{admin_token}}"})
    );
    assert_eq!(
        create_fixture.response.headers.get("set-cookie"),
        Some(&json!("single=one; Path=/"))
    );
    assert_eq!(recorded_fixture.request.path, "/datasets/{{dataset_id}}");
    assert_eq!(recorded_fixture.response.body, json!({"value": "stable"}));
    assert_eq!(
        recorded_fixture.response.headers.get("set-cookie"),
        Some(&json!(["first=one; Path=/", "second=two; Path=/"]))
    );
    assert!(!auth_fixture.response.headers.contains_key("set-cookie"));
    assert!(dataset_request.body.is_empty());
    assert!(
        requests
            .iter()
            .find(|request| request.url.path() == "/auth")
            .unwrap()
            .body
            .is_empty()
    );
    assert_eq!(passed.passed, 3);
    assert_eq!(passed.failed(), 0);
    assert!(passed.failures.is_empty());
    assert!(String::from_utf8_lossy(&help.stdout).contains("Usage: lens-parity"));
    assert!(cli_pass.status.success());
    assert!(String::from_utf8_lossy(&cli_pass.stdout).contains("3 passed, 0 failed"));

    server.reset().await;
    mount_responses(&server, "mutated").await;
    let failed = replay_fixtures(&parsed_base_url, fixtures.path(), &tokens)
        .await
        .unwrap();

    assert_eq!(failed.passed, 2);
    assert_eq!(failed.failed(), 1);
    assert_eq!(failed.failures[0].mismatches[0].path, "$.body.value");
    let cli_failure = Command::new(env!("CARGO_BIN_EXE_lens-parity"))
        .args(["replay", "--base-url"])
        .arg(&base_url)
        .arg("--fixtures")
        .arg(fixtures.path())
        .output()
        .unwrap();
    assert!(!cli_failure.status.success());
    assert!(String::from_utf8_lossy(&cli_failure.stdout).contains("2 passed, 1 failed"));
    assert!(String::from_utf8_lossy(&cli_failure.stdout).contains("$.body.value"));
}
