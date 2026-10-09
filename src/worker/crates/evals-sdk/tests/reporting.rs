use lens_evals_sdk::{
    model::{EvalRun, Report},
    reporting::{markdown, pass_rate_confidence, safe_text},
};
use rstest::rstest;

#[rstest]
#[case::link(
    "[click](https://attacker.example)",
    r"\[click\](https://attacker.example)"
)]
#[case::escaped_link(
    r"\[click\](https://attacker.example)",
    r"\\\[click\\\](https://attacker.example)"
)]
#[case::backslash(r"path\file", r"path\\file")]
fn report_text_cannot_reactivate_escaped_markdown(#[case] text: &str, #[case] expected: &str) {
    assert_eq!(safe_text(text), expected);
}

#[rstest]
#[case::all_pass(4, 4, 0.5101, 1.0)]
#[case::all_fail(0, 4, 0.0, 0.4899)]
#[case::mixed(2, 4, 0.1500, 0.8500)]
fn confidence_accounts_for_small_sample_uncertainty(
    #[case] passed: usize,
    #[case] total: usize,
    #[case] lower: f64,
    #[case] upper: f64,
) {
    let confidence = pass_rate_confidence(passed, total).unwrap();
    assert!((confidence.lower - lower).abs() < 0.0001);
    assert!((confidence.upper - upper).abs() < 0.0001);
}

#[rstest]
#[case::empty(0, 0)]
#[case::invalid(5, 4)]
fn confidence_rejects_invalid_case_counts(#[case] passed: usize, #[case] total: usize) {
    assert!(pass_rate_confidence(passed, total).is_err());
}

#[rstest]
fn comparison_discloses_confidence_method_and_links_both_builds() {
    let baseline: EvalRun = serde_json::from_str(include_str!(
        "../../../../sdk/tests/fixtures/lens_eval/eval_run_no_baseline.json"
    ))
    .unwrap();
    let mut candidate = baseline.clone();
    candidate.id = "candidate".into();
    candidate.version = "candidate-sha".into();
    candidate.url = "https://lens.example/runs/candidate".into();
    candidate.pr = Some(7);
    candidate.ci_url = "https://github.com/org/repo/actions/runs/99".into();
    let summary = candidate.summary.as_mut().unwrap();
    summary.baseline_run_id = Some(baseline.id.clone());
    summary.baseline_version = Some(baseline.version.clone());
    summary.passed = 18;
    summary.pass_rate = 0.5;
    let body = markdown(&Report {
        run: candidate,
        baseline: Some(baseline),
        trials: vec![],
    })
    .unwrap();
    assert!(body.contains("| Before / main | After / PR |"));
    assert!(body.contains("| Cases passed | 36/36 | 18/36 |"));
    assert!(body.contains("| Confidence score (95% CI lower bound) | 90.4% | 34.5% |"));
    assert!(body.contains("Pass-rate change: -50.0 percentage points"));
    assert!(body.contains("Repeats are not counted as new cases"));
    assert!(body.contains("not a probability of correctness or proof of improvement"));
    assert!(body.contains("[base](http://localhost:8765/runs/baseline)"));
    assert!(body.contains("[candidate-sha](https://lens.example/runs/candidate)"));
    assert!(body.contains("[Workflow logs](https://github.com/org/repo/actions/runs/99)"));
}
