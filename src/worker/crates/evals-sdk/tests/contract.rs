mod support;
use lens_evals_sdk::{engine, model::*, reporting};
use rstest::rstest;
use serde_json::Value;
use support::{Server, context, good, server, spec};

fn fixture(name: &str) -> Value {
    let runtime =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../contract/fixtures/lens_eval");
    let root = if runtime.exists() {
        runtime
    } else {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../sdk/tests/fixtures/lens_eval")
    };
    serde_json::from_str(&std::fs::read_to_string(root.join(format!("{name}.json"))).unwrap())
        .unwrap()
}

#[rstest]
#[case::create("create_run")]
#[case::trace("case_result_trace")]
#[case::error("case_result_error")]
#[case::done("eval_run_done")]
#[case::no_baseline("eval_run_no_baseline")]
fn golden_fixtures_round_trip(#[case] name: &str) {
    let input = fixture(name);
    let output = match name {
        "create_run" => {
            serde_json::to_value(serde_json::from_value::<CreateEvalRun>(input.clone()).unwrap())
                .unwrap()
        }
        "case_result_trace" | "case_result_error" => {
            serde_json::to_value(serde_json::from_value::<CaseResult>(input.clone()).unwrap())
                .unwrap()
        }
        _ => {
            serde_json::to_value(serde_json::from_value::<EvalRun>(input.clone()).unwrap()).unwrap()
        }
    };
    for (key, value) in input.as_object().unwrap() {
        assert_eq!(&output[key], value);
    }
}

#[rstest]
#[case::tie(2, 1, 1)]
#[case::majority(3, 2, 1)]
#[case::missing(3, 0, 3)]
#[tokio::test]
async fn majority_missing_and_closed_results(
    #[future] server: Server,
    #[case] trials: usize,
    #[case] submitted: usize,
    #[case] errors: usize,
) {
    let server = server.await;
    let body = CreateEvalRun {
        trials,
        case_ids: Some(vec!["case-0".into()]),
        ..serde_json::from_value(fixture("create_run")).unwrap()
    };
    let run = server.client.create(&body, "single").await.unwrap();
    let result = good();
    futures_util::future::try_join_all(
        (0..submitted).map(|trial| server.client.result(&run.id, "case-0", trial, &result)),
    )
    .await
    .unwrap();
    server.client.finish(&run.id).await.unwrap();
    let done = server.client.get(&run.id, false).await.unwrap();
    let summary = done.summary.unwrap();
    assert_eq!(summary.errors, errors);
    assert_eq!(summary.passed, usize::from(submitted * 2 > trials));
    assert!(
        server
            .client
            .result(&run.id, "case-0", 0, &good())
            .await
            .is_err()
    );
}

#[rstest]
#[case::same(false)]
#[case::changed(true)]
#[tokio::test]
async fn baseline_requires_matching_scorers(
    #[future] server: Server,
    spec: EvalSpec,
    #[case] changed: bool,
) {
    let server = server.await;
    engine::evaluate(
        &server.client,
        &spec,
        "demo",
        &context("main", "base"),
        |_| async { good() },
    )
    .await
    .unwrap();
    let candidate = EvalSpec {
        scores: if changed {
            vec![Scorer::Judge {
                prompt: "Does it work?".into(),
                model: String::new(),
            }]
        } else {
            spec.scores.clone()
        },
        ..spec
    };
    let report = engine::evaluate(
        &server.client,
        &candidate,
        "demo",
        &context("feature", "pr"),
        |_| async { good() },
    )
    .await
    .unwrap();
    assert_eq!(report.summary().unwrap().baseline_run_id.is_none(), changed);
}

#[rstest]
#[case::neutral(false, true, "neutral")]
#[case::fail(true, false, "failure")]
#[case::pass(true, true, "success")]
fn github_uses_authoritative_gate(
    #[case] baseline: bool,
    #[case] passed: bool,
    #[case] expected: &str,
) {
    let mut run: EvalRun = serde_json::from_value(fixture("eval_run_done")).unwrap();
    let summary = run.summary.as_mut().unwrap();
    summary.gate.passed = passed;
    summary.baseline_run_id = baseline.then(|| "baseline".into());
    let report = Report {
        run,
        baseline: None,
        trials: vec![],
    };
    assert_eq!(reporting::conclusion(&report).unwrap(), expected);
}

#[rstest]
#[case::huge_timeout(f64::MAX)]
#[case::zero_timeout(0.0)]
fn rejects_unrepresentable_timeout(spec: EvalSpec, #[case] timeout: f64) {
    assert!(
        EvalSpec {
            timeout_seconds: timeout,
            ..spec
        }
        .validate()
        .is_err()
    );
}
