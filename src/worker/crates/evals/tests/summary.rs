mod support;

use std::collections::BTreeMap;

use lens_contract::eval::{CalledBefore, CaseDiff, Gate, Scorer, TaskCompleted};
use lens_evals::{
    Error, GateFacts, RunInput, SpanStatus, Trial, TrialOutcome, check_gate, evaluate, majority,
};
use rstest::rstest;
use support::{FakeJudge, baseline, case, error, fail, judge_scorer, pass, root, run, tool};

fn judge() -> FakeJudge {
    FakeJudge::constant(1.0)
}

#[rstest]
#[case::one_of_one(1, 1, true)]
#[case::zero_of_one(0, 1, false)]
#[case::tie_one_of_two(1, 2, false)]
#[case::two_of_two(2, 2, true)]
#[case::two_of_three(2, 3, true)]
#[case::one_of_three(1, 3, false)]
#[case::tie_two_of_four(2, 4, false)]
fn majority_ties_fail(#[case] passing: usize, #[case] trials: usize, #[case] expected: bool) {
    assert_eq!(majority(passing, trials), expected);
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
        trace_spend_usd: 0.0,
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

#[rstest]
#[case::cost_usd_wins(Some(0.25), 9.0, 0.5)]
#[case::trace_spend_fallback(None, 0.125, 0.25)]
#[tokio::test]
async fn cost_per_case_sums_trials_over_cases(
    #[case] cost_usd: Option<f64>,
    #[case] spend: f64,
    #[case] expected: f64,
) {
    let trial = Trial {
        cost_usd,
        trace_spend_usd: spend,
        ..pass()
    };
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
    assert_eq!(summary.regressions[0].title, "c");
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
#[tokio::test]
async fn empty_runs_are_rejected(#[case] input: RunInput) {
    assert!(matches!(
        evaluate(&input, &judge()).await,
        Err(Error::EmptyRun)
    ));
}

#[tokio::test]
async fn judge_failure_aborts_evaluate() {
    let input = RunInput {
        scorers: vec![judge_scorer("p")],
        ..run(1, vec![case("c", false, vec![pass()])], None)
    };
    assert!(matches!(
        evaluate(&input, &FakeJudge(BTreeMap::new())).await,
        Err(Error::Judge(_))
    ));
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

fn facts(scores: &BTreeMap<String, f64>) -> GateFacts<'_> {
    GateFacts {
        has_baseline: true,
        revision: 4,
        regressions: 3,
        critical_regressions: 1,
        pass_rate: 0.8,
        cost_per_case: 0.2,
        scores,
    }
}

fn gate(build: fn(&mut Gate)) -> Gate {
    let mut gate = Gate {
        regressions: None,
        critical: None,
        ..Gate::default()
    };
    build(&mut gate);
    gate
}

#[rstest]
#[case::regressions_over(gate(|g| g.regressions = Some(0)), vec!["3 regressions (max 0)"])]
#[case::regressions_at_max(gate(|g| g.regressions = Some(3)), vec![])]
#[case::critical_over(gate(|g| g.critical = Some(0)), vec!["1 critical regressions (max 0)"])]
#[case::critical_at_max(gate(|g| g.critical = Some(1)), vec![])]
#[case::pass_rate_below(gate(|g| g.pass_rate = Some(0.9)), vec!["Pass rate below minimum"])]
#[case::pass_rate_equal(gate(|g| g.pass_rate = Some(0.8)), vec![])]
#[case::cost_above(gate(|g| g.cost_per_case = Some(0.1)), vec!["Cost per case above maximum"])]
#[case::cost_equal(gate(|g| g.cost_per_case = Some(0.2)), vec![])]
#[case::min_below(
    gate(|g| { g.min.insert("judge".into(), 0.95); }),
    vec!["judge below minimum 0.95"]
)]
#[case::min_equal(gate(|g| { g.min.insert("judge".into(), 0.9); }), vec![])]
#[case::min_missing_score(
    gate(|g| { g.min.insert("other".into(), 1.0); }),
    vec!["other below minimum 1"]
)]
#[case::all_in_order(
    gate(|g| {
        g.min.insert("z".into(), 1.0);
        g.min.insert("judge".into(), 1.0);
        g.cost_per_case = Some(0.0);
        g.pass_rate = Some(1.0);
        g.critical = Some(0);
        g.regressions = Some(0);
    }),
    vec![
        "3 regressions (max 0)",
        "1 critical regressions (max 0)",
        "Pass rate below minimum",
        "Cost per case above maximum",
        "judge below minimum 1",
        "z below minimum 1",
    ]
)]
fn gate_conditions(#[case] gate: Gate, #[case] reasons: Vec<&str>) {
    let scores = BTreeMap::from([("judge".to_owned(), 0.9)]);
    let result = check_gate(&gate, &facts(&scores));
    assert_eq!(result.passed, reasons.is_empty());
    assert_eq!(result.reasons, reasons);
}

#[rstest]
#[case::regressions_ignored(gate(|g| g.regressions = Some(0)), true, vec!["no baseline on main for rev 4"])]
#[case::critical_ignored(gate(|g| g.critical = Some(0)), true, vec!["no baseline on main for rev 4"])]
#[case::pass_rate_still_applies(
    gate(|g| g.pass_rate = Some(0.9)),
    false,
    vec!["Pass rate below minimum", "no baseline on main for rev 4"]
)]
fn gate_without_baseline(#[case] gate: Gate, #[case] passed: bool, #[case] reasons: Vec<&str>) {
    let scores = BTreeMap::new();
    let result = check_gate(
        &gate,
        &GateFacts {
            has_baseline: false,
            ..facts(&scores)
        },
    );
    assert_eq!(result.passed, passed);
    assert_eq!(result.reasons, reasons);
}

#[test]
fn gate_defaults_match_contract() {
    let gate: Gate = serde_json::from_str("{}").unwrap();
    assert_eq!(gate, Gate::default());
    assert_eq!((gate.regressions, gate.critical), (Some(0), Some(0)));
}
