#[path = "evals/support.rs"]
mod support;

use lens_contract::eval::EvalRun;
use rstest::rstest;
use support::{Fixture, fixture, normalized};

#[rstest]
#[tokio::test]
async fn golden_runs_match_dev_server_through_authenticated_routes_and_real_storage(
    #[future(awt)] fixture: Fixture,
) {
    let baseline = fixture.production_run("base", "main", false).await;
    let dev_baseline = fixture.dev_run("base", "main", false).await;
    let golden: EvalRun = serde_json::from_str(include_str!(
        "../../contract/fixtures/lens_eval/eval_run_no_baseline.json"
    ))
    .unwrap();
    assert_eq!(normalized(&baseline, None), normalized(&dev_baseline, None));
    assert_eq!(normalized(&baseline, None), golden.summary.unwrap());

    let candidate = fixture.production_run("candidate", "topic", false).await;
    let dev_candidate = fixture.dev_run("candidate", "topic", false).await;
    let golden: EvalRun = serde_json::from_str(include_str!(
        "../../contract/fixtures/lens_eval/eval_run_done.json"
    ))
    .unwrap();
    assert_eq!(
        normalized(&candidate, Some(&baseline)),
        normalized(&dev_candidate, Some(&dev_baseline))
    );
    assert_eq!(
        normalized(&candidate, Some(&baseline)),
        golden.summary.unwrap()
    );

    let broken = fixture.production_run("broken", "topic", true).await;
    let dev_broken = fixture.dev_run("broken", "topic", true).await;
    let summary = normalized(&broken, Some(&baseline));
    assert_eq!(summary, normalized(&dev_broken, Some(&dev_baseline)));
    assert_eq!(summary.errors, 3);
    assert_eq!(summary.passed, 35);
    assert_eq!(summary.regressions.len(), 1);
    assert_eq!(summary.regressions[0].case_id, "case-0");
    assert!(summary.regressions[0].critical);
    assert!(!summary.gate.passed);
}
