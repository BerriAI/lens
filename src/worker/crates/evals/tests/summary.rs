mod support;

use std::collections::BTreeMap;

use lens_contract::eval::{CalledBefore, CaseDiff, Gate, Scorer, TaskCompleted};
use lens_evals::{Error, RunInput, SpanStatus, Trial, TrialOutcome, evaluate, is_critical};
use rstest::rstest;
use support::{FakeJudge, baseline, case, error, fail, judge_scorer, pass, root, run, tool};

fn judge() -> FakeJudge {
    FakeJudge::constant(1.0)
}

#[rstest]
#[case::all_pass(vec![pass(), pass(), pass()], true, 0)]
#[case::two_of_three(vec![pass(), fail(), pass()], true, 0)]
#[case::errors_fail(vec![pass(), error(), error()], false, 2)]
#[case::error_breaks_majority(vec![pass(), pass(), error()], true, 1)]
#[case::missing_trials_fail(vec![pass()], false, 2)]
#[case::missing_and_error(vec![pass(), error()], false, 2)]
#[tokio::test]
async fn case_verdict_and_errors(
    #[case] trials: Vec<Trial>,
    #[case] passed: bool,
    #[case] errors: u64,
) {
    let result = evaluate(&run(3, vec![case("c", false, trials)], None), &judge())
        .await
        .unwrap();
    assert_eq!(result.verdicts["c"], passed);
    assert_eq!(result.summary.passed, u64::from(passed));
    assert_eq!(result.summary.errors, errors);
}

#[rstest]
#[case::all_scorers_pass(1.0, true)]
#[case::judge_fails_trial(0.2, false)]
#[tokio::test]
async fn trial_needs_every_scorer(#[case] judge_score: f64, #[case] passed: bool) {
    let input = RunInput {
        scorers: vec![Scorer::TaskCompleted(TaskCompleted {}), judge_scorer("p")],
        ..run(1, vec![case("c", false, vec![pass()])], None)
    };
    let result = evaluate(&input, &FakeJudge::constant(judge_score))
        .await
        .unwrap();
    assert_eq!(result.verdicts["c"], passed);
    assert_eq!(result.summary.scores["task_completed"], 1.0);
    assert_eq!(result.summary.scores["judge"], f64::from(u8::from(passed)));
}

#[tokio::test]
async fn scores_are_per_scorer_trial_means() {
    let spans = |tools: Vec<lens_evals::EvalSpan>| Trial {
        outcome: TrialOutcome::Trace([vec![root(SpanStatus::Ok)], tools].concat()),
        cost_usd: Some(0.0),
        trace_spend_usd: None,
    };
    let input = RunInput {
        scorers: vec![
            Scorer::TaskCompleted(TaskCompleted {}),
            Scorer::CalledBefore(CalledBefore {
                first: "a".into(),
                then: "b".into(),
            }),
            judge_scorer("first"),
            judge_scorer("second"),
        ],
        ..run(
            2,
            vec![
                case("x", false, vec![spans(vec![tool("t", "b", 1)]), error()]),
                case("y", false, vec![spans(vec![]), spans(vec![])]),
            ],
            None,
        )
    };
    let judge = FakeJudge(BTreeMap::from([
        ("first".to_owned(), 0.5),
        ("second".to_owned(), 0.4),
    ]));
    let summary = evaluate(&input, &judge).await.unwrap().summary;
    assert_eq!(
        summary.scores,
        BTreeMap::from([
            ("called_before".to_owned(), 0.5),
            ("judge_1".to_owned(), 0.75),
            ("judge_2".to_owned(), 0.0),
            ("task_completed".to_owned(), 0.75),
        ])
    );
    assert_eq!(summary.passed, 0);
    assert_eq!(summary.errors, 1);
}

fn priced(outcome: TrialOutcome, cost_usd: Option<f64>, spend: Option<f64>) -> Trial {
    Trial {
        outcome,
        cost_usd,
        trace_spend_usd: spend,
    }
}

fn traced() -> TrialOutcome {
    TrialOutcome::Trace(vec![root(SpanStatus::Ok)])
}

#[rstest]
#[case::cost_usd_set(priced(traced(), Some(0.25), None), 0.5)]
#[case::cost_usd_wins_over_spend(priced(traced(), Some(0.25), Some(9.0)), 0.5)]
#[case::spend_when_cost_usd_missing(priced(traced(), None, Some(0.125)), 0.25)]
#[case::neither_present(priced(traced(), None, None), 0.0)]
#[case::error_trial_cost_usd(priced(TrialOutcome::Error, Some(0.25), None), 0.5)]
#[case::error_trial_ignores_spend(priced(TrialOutcome::Error, None, Some(9.0)), 0.0)]
#[case::zero_cost_usd_wins_over_spend(priced(traced(), Some(0.0), Some(9.0)), 0.0)]
#[tokio::test]
async fn cost_per_case_sums_trials_over_cases(#[case] trial: Trial, #[case] expected: f64) {
    let input = run(
        2,
        vec![
            case("a", false, vec![trial.clone(), trial.clone()]),
            case("b", false, vec![trial.clone(), trial]),
        ],
        None,
    );
    let summary = evaluate(&input, &judge()).await.unwrap().summary;
    assert!((summary.cost_per_case - expected).abs() < 1e-12);
}

#[tokio::test]
async fn no_baseline_skips_diffs_and_regression_gates() {
    let input = run(1, vec![case("a", true, vec![fail()])], None);
    let summary = evaluate(&input, &judge()).await.unwrap().summary;
    assert_eq!(summary.baseline_run_id, None);
    assert_eq!(summary.baseline_version, None);
    assert!(summary.regressions.is_empty() && summary.fixed.is_empty());
    assert!(summary.gate.passed);
    assert_eq!(summary.gate.reasons, vec!["no baseline on main for rev 1"]);
}

#[tokio::test]
async fn no_baseline_reason_names_the_run_revision() {
    let input = RunInput {
        revision: 7,
        ..run(1, vec![case("a", false, vec![pass()])], None)
    };
    let summary = evaluate(&input, &judge()).await.unwrap().summary;
    assert_eq!(summary.gate.reasons, vec!["no baseline on main for rev 7"]);
}

#[tokio::test]
async fn regressions_fixed_and_critical_follow_case_order() {
    let input = run(
        1,
        vec![
            case("c", true, vec![fail()]),
            case("a", false, vec![fail()]),
            case("b", true, vec![pass()]),
            case("d", false, vec![pass()]),
            case("new", true, vec![fail()]),
        ],
        Some(baseline(&[
            ("a", true),
            ("b", false),
            ("c", true),
            ("d", true),
        ])),
    );
    let summary = evaluate(&input, &judge()).await.unwrap().summary;
    let ids = |diffs: &[CaseDiff]| {
        diffs
            .iter()
            .map(|diff| diff.case_id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&summary.regressions), vec!["c", "a"]);
    assert_eq!(ids(&summary.fixed), vec!["b"]);
    assert_eq!(summary.regressions[0].title, "Title c");
    assert!(summary.regressions[0].critical && !summary.regressions[1].critical);
    assert_eq!(
        summary.regressions[0].baseline_url,
        "http://lens/runs/baseline?case=c"
    );
    assert_eq!(
        summary.regressions[0].candidate_url,
        "http://lens/runs/candidate?case=c"
    );
    assert_eq!(summary.baseline_run_id.as_deref(), Some("baseline"));
    assert_eq!(summary.baseline_version.as_deref(), Some("base"));
    assert!(!summary.gate.passed);
    assert_eq!(
        summary.gate.reasons,
        vec!["2 regressions (max 0)", "1 critical regressions (max 0)"]
    );
}

#[rstest]
#[case::no_cases(run(1, vec![], None))]
#[case::zero_trials(run(0, vec![case("a", false, vec![])], None))]
#[case::no_scorers(RunInput { scorers: vec![], ..run(1, vec![case("a", false, vec![pass()])], None) })]
#[tokio::test]
async fn empty_runs_are_rejected(#[case] input: RunInput) {
    assert!(matches!(
        evaluate(&input, &judge()).await,
        Err(Error::EmptyRun)
    ));
}

#[tokio::test]
async fn judge_failure_is_an_error_trial_only() {
    let input = RunInput {
        scorers: vec![judge_scorer("p")],
        ..run(
            3,
            vec![
                case("c", false, vec![pass(), pass(), pass()]),
                case("d", false, vec![pass(), pass(), pass()]),
            ],
            None,
        )
    };
    let judge = support::FlakyJudge::failing(&[("c", 1)]);
    let result = evaluate(&input, &judge).await.unwrap();
    assert!(result.verdicts["c"] && result.verdicts["d"]);
    assert_eq!(result.summary.errors, 1);
    assert_eq!(result.summary.scores["judge"], 5.0 / 6.0);
}

#[tokio::test]
async fn duplicate_case_ids_are_rejected() {
    let input = run(
        1,
        vec![
            case("a", false, vec![pass()]),
            case("a", false, vec![fail()]),
        ],
        None,
    );
    assert!(matches!(
        evaluate(&input, &judge()).await,
        Err(Error::DuplicateCase { case_id }) if case_id == "a"
    ));
}

#[tokio::test]
async fn extra_trials_are_rejected() {
    let input = run(1, vec![case("a", false, vec![pass(), pass()])], None);
    assert!(matches!(
        evaluate(&input, &judge()).await,
        Err(Error::ExtraTrials {
            received: 2,
            expected: 1,
            ..
        })
    ));
}

#[test]
fn gate_defaults_match_contract() {
    let gate: Gate = serde_json::from_str("{}").unwrap();
    assert_eq!(gate, Gate::default());
    assert_eq!((gate.regressions, gate.critical), (Some(0), Some(0)));
}

#[rstest]
#[case::nan_spend(None, Some(f64::NAN))]
#[case::negative_spend(None, Some(-1.0))]
#[case::infinite_cost(Some(f64::INFINITY), None)]
#[case::negative_cost(Some(-0.5), None)]
#[tokio::test]
async fn invalid_costs_are_rejected(#[case] cost_usd: Option<f64>, #[case] spend: Option<f64>) {
    let trial = Trial {
        cost_usd,
        trace_spend_usd: spend,
        ..pass()
    };
    let input = run(1, vec![case("a", false, vec![trial])], None);
    assert!(matches!(
        evaluate(&input, &judge()).await,
        Err(Error::InvalidCost { case_id }) if case_id == "a"
    ));
}

#[rstest]
#[case::high(&[("priority", "high")], true)]
#[case::low(&[("priority", "low")], false)]
#[case::missing(&[("finding_id", "1")], false)]
#[case::case_sensitive(&[("priority", "HIGH")], false)]
fn critical_comes_from_high_priority(#[case] meta: &[(&str, &str)], #[case] expected: bool) {
    let meta: BTreeMap<String, String> = meta
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();
    assert_eq!(is_critical(&meta), expected);
}
