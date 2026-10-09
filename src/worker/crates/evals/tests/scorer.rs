mod support;

use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering::SeqCst},
};

use lens_evals::{
    EvalSpan, JudgeError, JudgeRequest, RunInput, SpanStatus, Trial, TrialOutcome, evaluate,
};
use rstest::rstest;
use support::{FakeJudge, case, judge_scorer, pass, root, run};

async fn judged(score: f64) -> lens_evals::Evaluation {
    let input = RunInput {
        scorers: vec![judge_scorer("p")],
        ..run(1, vec![case("c", false, vec![pass()])], None)
    };
    evaluate(&input, &FakeJudge::constant(score)).await.unwrap()
}

#[rstest]
#[case::below(0.49, false)]
#[case::at_threshold(0.5, true)]
#[case::above(0.9, true)]
#[case::zero(0.0, false)]
#[case::one(1.0, true)]
#[tokio::test]
async fn judge_threshold(#[case] score: f64, #[case] expected: bool) {
    let result = judged(score).await;
    assert_eq!(result.verdicts["c"], expected);
    assert_eq!(result.summary.errors, 0);
}

#[rstest]
#[case::negative(-0.1)]
#[case::above_one(1.5)]
#[case::nan(f64::NAN)]
#[tokio::test]
async fn out_of_range_judge_score_is_an_error_trial(#[case] score: f64) {
    let result = judged(score).await;
    assert!(!result.verdicts["c"]);
    assert_eq!(result.summary.errors, 1);
    assert_eq!(result.summary.scores["judge"], 0.0);
}

type Call = (String, usize, String, String, usize);

struct Recording(Mutex<Vec<Call>>);

impl lens_evals::Judge for Recording {
    async fn score(&self, request: JudgeRequest<'_>) -> Result<f64, JudgeError> {
        self.0.lock().unwrap().push((
            request.case_id.to_owned(),
            request.trial,
            request.prompt.to_owned(),
            request.model.to_owned(),
            request.spans.len(),
        ));
        Ok(1.0)
    }
}

#[tokio::test]
async fn judge_request_carries_case_trial_prompt_model_and_spans() {
    let input = RunInput {
        scorers: vec![judge_with_model("grade it", "gpt-x")],
        ..run(2, vec![case("c", false, vec![pass(), pass()])], None)
    };
    let judge = Recording(Mutex::new(Vec::new()));
    evaluate(&input, &judge).await.unwrap();
    let mut calls = judge.0.into_inner().unwrap();
    calls.sort();
    assert_eq!(
        calls,
        vec![
            ("c".into(), 0, "grade it".into(), "gpt-x".into(), 1),
            ("c".into(), 1, "grade it".into(), "gpt-x".into(), 1),
        ]
    );
}

fn judge_with_model(prompt: &str, model: &str) -> lens_contract::eval::Scorer {
    lens_contract::eval::Scorer::Judge(lens_contract::eval::Judge {
        prompt: prompt.into(),
        model: model.into(),
    })
}

#[rstest]
#[case::root_ok(vec![root(SpanStatus::Ok)], true, 0)]
#[case::root_error(vec![root(SpanStatus::Error)], false, 0)]
#[case::no_spans_is_error(vec![], false, 1)]
#[tokio::test]
async fn task_completed_through_evaluate(
    #[case] spans: Vec<EvalSpan>,
    #[case] passed: bool,
    #[case] errors: u64,
) {
    let trial = Trial {
        outcome: TrialOutcome::Trace(spans),
        cost_usd: None,
        trace_spend_usd: None,
    };
    let result = evaluate(
        &run(1, vec![case("c", false, vec![trial])], None),
        &FakeJudge::constant(1.0),
    )
    .await
    .unwrap();
    assert_eq!(result.verdicts["c"], passed);
    assert_eq!(result.summary.errors, errors);
}

#[rstest]
#[case::lower("\"error\"", SpanStatus::Error)]
#[case::clickhouse("\"STATUS_CODE_OK\"", SpanStatus::Ok)]
fn span_status_accepts_clickhouse_alias(#[case] json: &str, #[case] expected: SpanStatus) {
    assert_eq!(serde_json::from_str::<SpanStatus>(json).unwrap(), expected);
}

struct InFlight {
    current: AtomicUsize,
    peak: AtomicUsize,
}

impl lens_evals::Judge for InFlight {
    async fn score(&self, _request: JudgeRequest<'_>) -> Result<f64, JudgeError> {
        let now = self.current.fetch_add(1, SeqCst) + 1;
        self.peak.fetch_max(now, SeqCst);
        tokio::task::yield_now().await;
        self.current.fetch_sub(1, SeqCst);
        Ok(1.0)
    }
}

#[tokio::test]
async fn judge_calls_overlap() {
    let input = RunInput {
        scorers: vec![judge_scorer("p")],
        ..run(
            3,
            vec![case("c", false, vec![pass(), pass(), pass()])],
            None,
        )
    };
    let judge = InFlight {
        current: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
    };
    evaluate(&input, &judge).await.unwrap();
    assert!(judge.peak.into_inner() > 1);
}
