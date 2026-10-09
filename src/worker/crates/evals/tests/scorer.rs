mod support;

use lens_evals::{Error, EvalSpan, Scorer, SpanStatus, scorer, scorer_keys};
use rstest::rstest;
use support::{FakeJudge, root, span, tool};

#[rstest]
#[case::never_called(vec![tool("a", "search", 1)], true)]
#[case::no_tools_at_all(vec![], true)]
#[case::first_missing(vec![tool("a", "write", 1)], false)]
#[case::then_before_first(vec![tool("a", "write", 1), tool("b", "search", 2)], false)]
#[case::first_then_ordered(vec![tool("a", "search", 1), tool("b", "write", 2)], true)]
#[case::same_start(vec![tool("a", "search", 5), tool("b", "write", 5)], false)]
#[case::interleaved_ok(
    vec![tool("a", "search", 1), tool("b", "write", 2), tool("c", "search", 3), tool("d", "write", 4)],
    true
)]
#[case::interleaved_early_then(
    vec![tool("a", "write", 1), tool("b", "search", 2), tool("c", "write", 3)],
    false
)]
#[case::multiple_then_after_one_first(
    vec![tool("a", "search", 1), tool("b", "write", 2), tool("c", "write", 3)],
    true
)]
#[case::unordered_input(vec![tool("b", "write", 9), tool("a", "search", 3)], true)]
#[case::span_name_is_not_tool(
    vec![span("search", "root", 1, SpanStatus::Ok), tool("b", "write", 2)],
    false
)]
fn called_before_edges(#[case] spans: Vec<EvalSpan>, #[case] expected: bool) {
    assert_eq!(scorer::called_before(&spans, "search", "write"), expected);
}

#[rstest]
#[case::ok(vec![root(SpanStatus::Ok)], true)]
#[case::unset(vec![root(SpanStatus::Unset)], true)]
#[case::error(vec![root(SpanStatus::Error)], false)]
#[case::no_spans(vec![], false)]
#[case::child_error_root_ok(
    vec![root(SpanStatus::Ok), span("child", "root", 1, SpanStatus::Error)],
    true
)]
#[case::orphan_is_root(vec![span("orphan", "gone", 0, SpanStatus::Ok)], true)]
#[case::empty_parent_is_root(
    vec![root(SpanStatus::Ok), span("", "gone", 1, SpanStatus::Error)],
    true
)]
#[case::earliest_root_wins(
    vec![span("late", "", 9, SpanStatus::Ok), span("early", "", 1, SpanStatus::Error)],
    false
)]
#[case::self_parent_is_root(
    vec![span("loop", "loop", 0, SpanStatus::Ok), span("child", "loop", 1, SpanStatus::Error)],
    true
)]
fn task_completed_reads_root_status(#[case] spans: Vec<EvalSpan>, #[case] expected: bool) {
    assert_eq!(scorer::task_completed(&spans), expected);
}

#[rstest]
#[case::below(0.49, false)]
#[case::at_threshold(0.5, true)]
#[case::above(0.9, true)]
#[case::zero(0.0, false)]
#[case::one(1.0, true)]
#[tokio::test]
async fn judge_threshold(#[case] score: f64, #[case] expected: bool) {
    let judge = Scorer::Judge {
        prompt: "p".into(),
        model: String::new(),
    };
    let passed = judge
        .passes("c", &[root(SpanStatus::Ok)], &FakeJudge::constant(score))
        .await
        .unwrap();
    assert_eq!(passed, expected);
}

#[rstest]
#[case::negative(-0.1)]
#[case::above_one(1.5)]
#[case::nan(f64::NAN)]
#[tokio::test]
async fn judge_rejects_out_of_range(#[case] score: f64) {
    let judge = Scorer::Judge {
        prompt: "p".into(),
        model: String::new(),
    };
    let result = judge.passes("c", &[], &FakeJudge::constant(score)).await;
    assert!(matches!(result, Err(Error::JudgeScore { .. })));
}

#[tokio::test]
async fn judge_failure_propagates() {
    let judge = Scorer::Judge {
        prompt: "missing".into(),
        model: String::new(),
    };
    let result = judge.passes("c", &[], &FakeJudge(Default::default())).await;
    assert!(matches!(result, Err(Error::Judge(_))));
}

fn judge(prompt: &str) -> Scorer {
    Scorer::Judge {
        prompt: prompt.into(),
        model: String::new(),
    }
}

fn before() -> Scorer {
    Scorer::CalledBefore {
        first: "a".into(),
        then: "b".into(),
    }
}

#[rstest]
#[case::single(vec![Scorer::TaskCompleted], vec!["task_completed"])]
#[case::judges_numbered(
    vec![judge("x"), Scorer::TaskCompleted, judge("y")],
    vec!["judge_1", "task_completed", "judge_2"]
)]
#[case::repeated_called_before(vec![before(), before()], vec!["called_before_1", "called_before_2"])]
fn scorer_keys_follow_kind(#[case] scorers: Vec<Scorer>, #[case] expected: Vec<&str>) {
    assert_eq!(scorer_keys(&scorers), expected);
}

#[rstest]
#[case::task(r#"{"kind":"task_completed"}"#, Scorer::TaskCompleted)]
#[case::called_before(r#"{"kind":"called_before","first":"a","then":"b"}"#, before())]
#[case::judge_without_model(r#"{"kind":"judge","prompt":"x"}"#, judge("x"))]
fn scorer_matches_contract_shape(#[case] json: &str, #[case] expected: Scorer) {
    assert_eq!(serde_json::from_str::<Scorer>(json).unwrap(), expected);
}

#[rstest]
#[case::lower("\"error\"", SpanStatus::Error)]
#[case::clickhouse("\"STATUS_CODE_OK\"", SpanStatus::Ok)]
fn span_status_accepts_clickhouse_alias(#[case] json: &str, #[case] expected: SpanStatus) {
    assert_eq!(serde_json::from_str::<SpanStatus>(json).unwrap(), expected);
}
