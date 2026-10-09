mod support;

use axum::{
    Json,
    extract::Request,
    middleware::{self, Next},
    routing::get,
};
use lens_evals_sdk::{devserver, model::Execution, named};
use rstest::rstest;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    sync::{Arc, Mutex},
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

#[rstest]
#[tokio::test]
async fn saved_definition_creates_one_snapshotted_run_and_replay_does_not_submit_again() {
    let agent = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/run"))
        .and(header("Authorization", "Bearer agent-token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"output":"real fixture output","id":"pass-fixture"})),
        )
        .expect(4)
        .mount(&agent)
        .await;
    let io = json!({"version":1,"connection":"local","submit":{"method":"POST","path":"/run","accepted_status":200,"json":{"input":"","request_id":""}},"input":[{"source":"case.input","target":"/input"},{"source":"trial.request_id","target":"/request_id"}],"completion":{"kind":"immediate"},"output":{"pointer":"/output","require_nonempty":true},"trace":{"source":"accepted","attribute":"session.id","pointer":"/id"}});
    let definition = json!({"name":"demo","updated_at":"now","spec":{"agent":"saved-agent","dataset_id":"demo","revision":null,"scorers":[{"kind":"task_completed"}],"trials":2,"baseline":"main","gate":{"pass_rate":1},"timeout_per_trial_ms":3000,"agent_io":io}});
    let captured = Arc::new(Mutex::new(Vec::new()));
    let requests = captured.clone();
    let app = devserver::router(devserver::sample_cases(2), "lens-dev".into())
        .route(
            "/lens/evals/demo",
            get(move || {
                let definition = definition.clone();
                async move { Json(definition) }
            }),
        )
        .route(
            "/lens/datasets/demo",
            get(|| async { Json(json!({"id":"demo","name":"display-name","revision":1})) }),
        )
        .layer(middleware::from_fn(move |request: Request, next: Next| {
            let requests = requests.clone();
            async move {
                assert_eq!(request.headers()["X-Lens-Contract"], "2");
                let method = request.method().clone();
                let path = request.uri().path().to_owned();
                let (parts, body) = request.into_parts();
                let bytes = axum::body::to_bytes(body, 1024 * 1024).await.unwrap();
                let body = serde_json::from_slice::<Value>(&bytes).ok();
                requests.lock().unwrap().push((method, path, body));
                next.run(Request::from_parts(parts, axum::body::Body::from(bytes)))
                    .await
            }
        }));
    let server = support::serve(app).await;
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("pyproject.toml"), "[tool.lens]\nproject='ignored-local-name'\n[tool.lens.connections.local]\nauth='bearer'\nbase_url_env='URL'\ntoken_env='TOKEN'\n").unwrap();
    let environment = |name: &str| match name {
        "URL" => Some(agent.uri()),
        "TOKEN" => Some("agent-token".into()),
        _ => panic!("Remote data must not select an environment variable"),
    };
    let execution = Execution {
        version: "sha".into(),
        branch: "main".into(),
        pr: None,
        ci_url: String::new(),
        identity: "same-execution".into(),
    };
    let first = named::evaluate_with_environment(
        "demo",
        &server.address,
        "lens-dev",
        &execution,
        root.path(),
        environment,
    )
    .await
    .unwrap();
    let repeat = named::evaluate_with_environment(
        "demo",
        &server.address,
        "lens-dev",
        &execution,
        root.path(),
        environment,
    )
    .await
    .unwrap();
    assert_eq!(first.run.id, repeat.run.id);
    assert_eq!(first.run.agent, "saved-agent");
    assert_eq!(first.run.received_trials, 4);
    assert!(first.summary().unwrap().gate.passed);
    assert!(repeat.trials.is_empty());
    let submitted = agent.received_requests().await.unwrap();
    let ids = submitted
        .iter()
        .map(|request| {
            request.body_json::<Value>().unwrap()["request_id"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), 4);
    let requests = captured.lock().unwrap();
    let creations = requests
        .iter()
        .filter(|(method, path, _)| method == "POST" && path == "/lens/evals/runs")
        .collect::<Vec<_>>();
    assert_eq!(creations.len(), 2);
    let snapshot = creations[0].2.as_ref().unwrap();
    assert_eq!(snapshot["revision"], 1);
    assert_eq!(snapshot["agent_io"], io);
    assert_eq!(snapshot["case_ids"], json!(["case-0", "case-1"]));
    assert_eq!(snapshot["trials"], 2);
    assert_eq!(snapshot["gate"]["pass_rate"], 1.0);
    assert_eq!(
        requests
            .iter()
            .filter(|(_, path, _)| path == "/lens/datasets/demo")
            .count(),
        2
    );
    assert!(
        !requests
            .iter()
            .any(|(method, path, _)| method == "PUT" && path == "/lens/evals/demo")
    );
}
