mod support;

use lens_evals_sdk::{engine, model::*, reporting};
use reqwest::Method;
use rstest::rstest;
use serde_json::json;
use support::{Server, good, server};

#[rstest]
#[tokio::test]
async fn sdk_waits_after_task_returns_accepted_reference(#[future] server: Server) {
    let server = server.await;
    let client = server.client.clone();
    let evaluation = tokio::spawn(async move {
        let spec = EvalSpec {
            trials: 1,
            case_ids: Some(vec!["case-0".into()]),
            ..support::spec()
        };
        engine::evaluate(
            &client,
            &spec,
            "demo",
            &support::context("main", "async"),
            |_| async {
                CaseResult {
                    trace: Some(TraceRef {
                        attribute: "session.id".into(),
                        value: "accepted-pass".into(),
                    }),
                    ..CaseResult::default()
                }
            },
        )
        .await
        .unwrap()
    });
    let pending = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let response = server
                .client
                .request(Method::GET, "/lens/evals/runs", None, None, false)
                .await
                .unwrap();
            let runs: Vec<EvalRun> = lens_evals_sdk::client::Client::decode(response)
                .await
                .unwrap();
            if let Some(run) = runs
                .into_iter()
                .find(|run| matches!(run.status, RunStatus::Scoring))
            {
                break run;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(pending.received_trials, 1);
    assert!(!evaluation.is_finished());
    let update = json!({"trace":{"attribute":"session.id","value":"accepted-pass"},"agent_version":"sha","root_ended":true});
    server
        .client
        .request(Method::POST, "/_dev/traces", Some(&update), None, false)
        .await
        .unwrap();
    let report = evaluation.await.unwrap();
    assert_eq!(report.summary().unwrap().passed, 1);
    assert_eq!(report.summary().unwrap().errors, 0);
}

#[rstest]
#[case::matching_build("base", 0, 1)]
#[case::stale_build("stale", 1, 0)]
#[tokio::test]
async fn accepted_reference_waits_for_trace_and_checks_build(
    #[future] server: Server,
    #[case] version: &str,
    #[case] errors: usize,
    #[case] passed: usize,
) {
    let server = server.await;
    let body: CreateEvalRun = serde_json::from_value(json!({"eval":"async", "agent":"demo", "dataset_id":"demo", "revision":1, "version":"base", "branch":"main", "case_ids":["case-0"], "scorers":[{"kind":"task_completed"}]})).unwrap();
    let run = server.client.create(&body, "accepted").await.unwrap();
    let reference = TraceRef {
        attribute: "session.id".into(),
        value: "accepted-pass".into(),
    };
    server
        .client
        .result(
            &run.id,
            "case-0",
            0,
            &CaseResult {
                trace: Some(reference.clone()),
                ..CaseResult::default()
            },
        )
        .await
        .unwrap();
    server.client.finish(&run.id).await.unwrap();
    let pending = server.client.get(&run.id, false).await.unwrap();
    assert!(matches!(pending.status, RunStatus::Scoring));
    assert!(pending.summary.is_none());
    let update = json!({"trace": reference, "agent_version":version,"root_ended":true});
    server
        .client
        .request(Method::POST, "/_dev/traces", Some(&update), None, false)
        .await
        .unwrap();
    let done = server.client.get(&run.id, false).await.unwrap();
    assert!(matches!(done.status, RunStatus::Done));
    let summary = done.summary.unwrap();
    assert_eq!(summary.errors, errors);
    assert_eq!(summary.passed, passed);
}

#[rstest]
#[case::pass_rate(Gate { pass_rate: Some(1.0), ..Gate::default() }, true)]
#[case::cost(Gate { cost_per_case: Some(0.0), ..Gate::default() }, false)]
#[case::minimum(Gate { min: [("task_completed".into(), 1.0)].into(), ..Gate::default() }, true)]
#[tokio::test]
async fn failed_absolute_condition_is_red_without_baseline(
    #[future] server: Server,
    #[case] gate: Gate,
    #[case] bad_task: bool,
) {
    let server = server.await;
    let spec = EvalSpec {
        gate,
        ..support::spec()
    };
    let report = engine::evaluate(
        &server.client,
        &spec,
        "demo",
        &support::context("feature", "absolute"),
        |_| async {
            if bad_task {
                engine::failure("TaskError", "agent failed")
            } else {
                good()
            }
        },
    )
    .await
    .unwrap();
    assert!(report.summary().unwrap().baseline_run_id.is_none());
    assert!(!report.summary().unwrap().gate.passed);
    assert_eq!(reporting::conclusion(&report).unwrap(), "failure");
}

#[rstest]
#[case::metadata_only("7", "", "7", true)]
#[case::legacy_source("", "7", "7", true)]
#[case::metadata_wins("8", "7", "7", false)]
#[case::opaque_id("finding-7", "", "finding-7", true)]
fn finding_subset_uses_metadata_then_legacy_source(
    #[case] metadata: &str,
    #[case] source: &str,
    #[case] selector: &str,
    #[case] selected: bool,
) {
    let mut dataset = lens_evals_sdk::devserver::sample_cases(1);
    dataset.cases[0].meta.remove("finding_id");
    if !metadata.is_empty() {
        dataset.cases[0]
            .meta
            .insert("finding_id".into(), metadata.into());
    }
    dataset.cases[0].source.finding_id = source.into();
    let spec = EvalSpec {
        finding: Some(selector.into()),
        ..support::spec()
    };
    assert_eq!(spec.select(&dataset).is_ok(), selected);
}

#[rstest]
#[case::missing_trace(false)]
#[case::open_trace(true)]
#[tokio::test]
async fn custom_trial_timeout_reaches_server(#[future] server: Server, #[case] observed: bool) {
    let server = server.await;
    if observed {
        server.client.request(
            Method::POST,
            "/_dev/traces",
            Some(&json!({"trace":{"attribute":"session.id","value":"accepted-pass-timeout"},"agent_version":"sha","root_ended":false})),
            None,
            false,
        ).await.unwrap();
    }
    let spec = EvalSpec {
        trials: 1,
        timeout_seconds: 0.025,
        case_ids: Some(vec!["case-0".into()]),
        ..support::spec()
    };
    let report = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        engine::evaluate(
            &server.client,
            &spec,
            "demo",
            &support::context("feature", "timeout"),
            |_| async {
                CaseResult {
                    trace: Some(TraceRef {
                        attribute: "session.id".into(),
                        value: "accepted-pass-timeout".into(),
                    }),
                    ..CaseResult::default()
                }
            },
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(report.trials[0].result.error.is_none());
    assert_eq!(report.summary().unwrap().errors, 1);
    assert_eq!(report.summary().unwrap().passed, 0);
    assert_eq!(reporting::conclusion(&report).unwrap(), "failure");
}

#[rstest]
#[tokio::test]
async fn zero_trial_timeout_rejected(#[future] server: Server) {
    let server = server.await;
    let body: CreateEvalRun = serde_json::from_value(json!({"eval":"async", "agent":"demo", "dataset_id":"demo", "revision":1, "version":"base", "branch":"main", "scorers":[{"kind":"task_completed"}], "timeout_per_trial_ms":0})).unwrap();
    let error = server
        .client
        .create(&body, "zero-timeout")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("422"));
}
