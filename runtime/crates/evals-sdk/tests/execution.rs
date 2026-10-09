mod support;

use lens_evals_sdk::{Error, devserver, engine, model::*};
use rstest::rstest;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use support::{Server, context, good, server, spec};

#[rstest]
#[tokio::test]
async fn baseline_regression_revert_repeat(#[future] server: Server, spec: EvalSpec) {
    let server = server.await;
    let baseline = engine::evaluate(
        &server.client,
        &spec,
        "demo",
        &context("main", "base"),
        |_| async { good() },
    )
    .await
    .unwrap();
    assert_eq!(baseline.run.received_trials, 108);
    assert_eq!(baseline.summary().unwrap().passed, 36);
    let broken = engine::evaluate(
        &server.client,
        &spec,
        "demo",
        &context("feature", "broken"),
        |case| async move {
            if case.id == "case-0" {
                engine::failure("ValueError", "agent failed")
            } else {
                good()
            }
        },
    )
    .await
    .unwrap();
    assert!(!broken.summary().unwrap().gate.passed);
    assert_eq!(broken.summary().unwrap().regressions[0].case_id, "case-0");
    assert!(broken.summary().unwrap().regressions[0].critical);
    assert_eq!(
        broken.summary().unwrap().baseline_run_id.as_deref(),
        Some(baseline.run.id.as_str())
    );
    let reverted = engine::evaluate(
        &server.client,
        &spec,
        "demo",
        &context("feature", "revert"),
        |_| async { good() },
    )
    .await
    .unwrap();
    let repeat = engine::evaluate(
        &server.client,
        &spec,
        "demo",
        &context("feature", "repeat"),
        |_| async { good() },
    )
    .await
    .unwrap();
    assert!(reverted.summary().unwrap().gate.passed && repeat.summary().unwrap().gate.passed);
    assert!(repeat.summary().unwrap().regressions.is_empty());
    assert_ne!(reverted.run.id, repeat.run.id);
    assert!((repeat.summary().unwrap().cost_per_case - 0.03).abs() < 1e-9);
}

struct Active(Arc<AtomicUsize>);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[rstest]
#[tokio::test]
async fn bounded_concurrency_and_timeout_cancellation(#[future] server: Server, spec: EvalSpec) {
    let server = server.await;
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let spec = EvalSpec {
        timeout_seconds: 0.02,
        ..spec
    };
    let report = engine::evaluate(
        &server.client,
        &spec,
        "demo",
        &context("main", "timeout"),
        |_| {
            let active = active.clone();
            let maximum = maximum.clone();
            async move {
                let guard = Active(active.clone());
                maximum.fetch_max(active.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_secs(10)).await;
                drop(guard);
                good()
            }
        },
    )
    .await
    .unwrap();
    assert_eq!(maximum.load(Ordering::SeqCst), spec.concurrency);
    assert_eq!(active.load(Ordering::SeqCst), 0);
    assert_eq!(report.summary().unwrap().errors, 108);
    assert!(!report.summary().unwrap().gate.passed);
}

#[rstest]
#[case::case_ids(Some(vec!["case-0".into(), "case-1".into()]), None, 2)]
#[case::finding(None, Some("1".into()), 36)]
#[tokio::test]
async fn selects_cases(
    #[future] server: Server,
    spec: EvalSpec,
    #[case] case_ids: Option<Vec<String>>,
    #[case] finding: Option<String>,
    #[case] total: usize,
) {
    let server = server.await;
    let selected = EvalSpec {
        case_ids,
        finding,
        ..spec
    };
    let report = engine::evaluate(
        &server.client,
        &selected,
        "demo",
        &context("main", "subset"),
        |_| async { good() },
    )
    .await
    .unwrap();
    assert_eq!(report.summary().unwrap().total, total);
    assert_eq!(report.run.received_trials, total * selected.trials);
}

#[rstest]
#[case::create("/lens/evals/runs")]
#[case::finish("/finish")]
#[tokio::test]
async fn lost_write_ack_reuses_run_and_recovers_finish(spec: EvalSpec, #[case] path: &'static str) {
    use axum::{
        extract::Request,
        http::StatusCode,
        middleware::{self, Next},
        response::IntoResponse,
    };
    let attempts = Arc::new(AtomicUsize::new(0));
    let counted = attempts.clone();
    let app = devserver::router(devserver::sample_cases(2), "lens-dev".into()).layer(
        middleware::from_fn(move |request: Request, next: Next| {
            let counted = counted.clone();
            async move {
                let fail = request.method() == "POST"
                    && request.uri().path().ends_with(path)
                    && counted.fetch_add(1, Ordering::SeqCst) == 0;
                let response = next.run(request).await;
                if fail {
                    StatusCode::SERVICE_UNAVAILABLE.into_response()
                } else {
                    response
                }
            }
        }),
    );
    let server = support::serve(app).await;
    let report = engine::evaluate(
        &server.client,
        &spec,
        "demo",
        &context("main", "retry"),
        |_| async { good() },
    )
    .await
    .unwrap();
    assert!(report.summary().unwrap().gate.passed);
    assert_eq!(report.run.received_trials, 6);
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    let runs: Vec<EvalRun> = lens_evals_sdk::client::Client::decode(
        server
            .client
            .request(reqwest::Method::GET, "/lens/evals/runs", None, None, false)
            .await
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(runs.len(), 1);
}

#[rstest]
#[tokio::test]
async fn upload_failure_is_infrastructure(#[future] server: Server, spec: EvalSpec) {
    let server = server.await;
    let wrong = lens_evals_sdk::client::Client::new(&server.address, "wrong").unwrap();
    let error = engine::evaluate(
        &wrong,
        &spec,
        "demo",
        &context("main", "denied"),
        |_| async { panic!("must not call agent") },
    )
    .await
    .unwrap_err();
    assert!(matches!(error, Error::Api { status: 401, .. }));
}

#[rstest]
#[case::failed("failed", false)]
#[case::missing_summary("done", false)]
#[case::unknown_status("unknown", false)]
#[tokio::test]
async fn backend_failure_is_not_a_gate_failure(
    spec: EvalSpec,
    #[case] status: &str,
    #[case] task_called: bool,
) {
    use serde_json::json;
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
    let server = MockServer::start().await;
    Mock::given(path("/lens/datasets/resolve"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id":"demo","name":"demo","revision":1})),
        )
        .mount(&server)
        .await;
    Mock::given(path("/lens/datasets/demo/revisions/1/cases"))
        .respond_with(ResponseTemplate::new(200).set_body_json(devserver::sample_cases(1)))
        .mount(&server)
        .await;
    Mock::given(path("/lens/evals/runs")).respond_with(ResponseTemplate::new(201).set_body_json(json!({"id":"test","status":status,"eval":"demo","agent":"demo","version":"sha","branch":"main","pr":null,"url":"http://localhost/run","expected_trials":3,"received_trials":0,"summary":null,"failure":"backend unavailable"}))).mount(&server).await;
    let called = AtomicUsize::new(0);
    let result = engine::evaluate(
        &lens_evals_sdk::client::Client::new(&server.uri(), "key").unwrap(),
        &spec,
        "demo",
        &context("main", "failure"),
        |_| async {
            called.fetch_add(1, Ordering::SeqCst);
            good()
        },
    )
    .await;
    assert!(result.is_err());
    assert_eq!(called.load(Ordering::SeqCst) > 0, task_called);
}
