mod support;

use axum::{Json, routing::get};
use lens_evals_sdk::{
    devserver,
    model::{CaseError, CaseResult},
    session::Evaluation,
};
use rstest::rstest;
use serde_json::json;

async fn server(trials: usize) -> support::Server {
    let definition = json!({"name":"python-eval","updated_at":"now","spec":{"agent":"python-agent","dataset_id":"demo","revision":1,"scorers":[{"kind":"task_completed"}],"trials":trials,"baseline":"main","gate":{"pass_rate":1},"timeout_per_trial_ms":1000}});
    support::serve(
        devserver::router(devserver::sample_cases(2), "lens-dev".into()).route(
            "/lens/evals/python-eval",
            get(move || {
                let definition = definition.clone();
                async move { Json(definition) }
            }),
        ),
    )
    .await
}

#[rstest]
#[tokio::test]
async fn local_outputs_use_saved_cases_trials_and_server_baseline_without_an_http_agent() {
    let server = server(2).await;
    let context = support::context("main", "baseline");
    let mut baseline = Evaluation::start("python-eval", &server.address, "lens-dev", &context)
        .await
        .unwrap();
    assert_eq!(baseline.cases().len(), 4);
    for case in baseline.cases().to_vec() {
        baseline
            .record(
                &case,
                CaseResult {
                    output: Some(format!("Answer to {}", case.case.input)),
                    ..support::good()
                },
            )
            .await
            .unwrap();
    }
    let before = baseline.finish(None).await.unwrap();
    assert!(before.summary().unwrap().gate.passed);
    assert_eq!(before.run.agent, "python-agent");
    let mut replay = Evaluation::start("python-eval", &server.address, "lens-dev", &context)
        .await
        .unwrap();
    assert!(replay.cases().is_empty());
    assert_eq!(replay.finish(None).await.unwrap().run.id, before.run.id);
    let mut other = Evaluation::start(
        "python-eval",
        &server.address,
        "lens-dev",
        &support::context("main", "other-baseline"),
    )
    .await
    .unwrap();
    let latest = other.finish(None).await.unwrap();
    assert_ne!(latest.run.id, before.run.id);
    let execution = lens_evals_sdk::model::Execution {
        baseline_run_id: Some(before.run.id.clone()),
        ..support::context("topic", "candidate")
    };
    let mut candidate = Evaluation::start("python-eval", &server.address, "lens-dev", &execution)
        .await
        .unwrap();
    let case = candidate.cases()[0].clone();
    candidate.record(&case, support::good()).await.unwrap();
    let after = candidate
        .finish(Some(CaseError {
            r#type: "RuntimeError".into(),
            message: "Agent exited".into(),
        }))
        .await
        .unwrap();
    assert_eq!(after.run.received_trials, 4);
    assert_eq!(
        after
            .trials
            .iter()
            .filter(|trial| trial.result.error.is_some())
            .count(),
        3
    );
    assert_eq!(after.summary().unwrap().errors, 3);
    assert_eq!(after.baseline.as_ref().unwrap().id, before.run.id);
    assert!(!after.summary().unwrap().gate.passed);
    assert_eq!(candidate.finish(None).await.unwrap().run.id, after.run.id);
}

#[rstest]
#[tokio::test]
async fn rejects_changed_inputs_unknown_cases_duplicate_trials_and_closed_writes() {
    let server = server(1).await;
    let mut evaluation = Evaluation::start(
        "python-eval",
        &server.address,
        "lens-dev",
        &support::context("main", "validation"),
    )
    .await
    .unwrap();
    let case = evaluation.cases()[0].clone();
    let mut changed = case.clone();
    changed.case.input = "Different task".into();
    assert!(
        evaluation
            .record(&changed, support::good())
            .await
            .unwrap_err()
            .to_string()
            .contains("input")
    );
    changed.trial = 20;
    assert!(
        evaluation
            .record(&changed, support::good())
            .await
            .unwrap_err()
            .to_string()
            .contains("outside")
    );
    assert!(
        evaluation
            .record(&case, CaseResult::default())
            .await
            .is_err()
    );
    evaluation.record(&case, support::good()).await.unwrap();
    assert!(
        evaluation
            .record(&case, support::good())
            .await
            .unwrap_err()
            .to_string()
            .contains("already")
    );
    let report = evaluation.finish(None).await.unwrap();
    assert_eq!(report.summary().unwrap().errors, 1);
    assert_eq!(
        report.trials[1].result.error.as_ref().unwrap().r#type,
        "MissingResult"
    );
    assert!(
        evaluation
            .record(&case, support::good())
            .await
            .unwrap_err()
            .to_string()
            .contains("finishing")
    );
}
