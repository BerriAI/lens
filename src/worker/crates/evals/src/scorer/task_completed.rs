use super::{EvalSpan, SpanStatus};

pub(super) fn task_completed(spans: &[EvalSpan]) -> bool {
    let roots: Vec<&EvalSpan> = spans
        .iter()
        .filter(|span| span.parent_span_id.is_empty() || span.parent_span_id == span.span_id)
        .collect();
    !roots.is_empty() && roots.iter().all(|span| span.status != SpanStatus::Error)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn span(id: &str, parent: &str, status: SpanStatus) -> EvalSpan {
        EvalSpan {
            span_id: id.into(),
            parent_span_id: parent.into(),
            name: id.into(),
            start_ns: 0,
            status,
            tool_name: None,
        }
    }

    #[rstest]
    #[case::ok(vec![span("root", "", SpanStatus::Ok)], true)]
    #[case::unset(vec![span("root", "", SpanStatus::Unset)], true)]
    #[case::error(vec![span("root", "", SpanStatus::Error)], false)]
    #[case::no_spans(vec![], false)]
    #[case::child_error_root_ok(
        vec![span("root", "", SpanStatus::Ok), span("child", "root", SpanStatus::Error)],
        true
    )]
    #[case::orphan_is_not_root(vec![span("tool1", "agent-root", SpanStatus::Ok)], false)]
    #[case::orphan_error_ignored(
        vec![span("root", "", SpanStatus::Ok), span("tool1", "gone", SpanStatus::Error)],
        true
    )]
    #[case::self_parent_is_root(
        vec![span("loop", "loop", SpanStatus::Ok), span("child", "loop", SpanStatus::Error)],
        true
    )]
    #[case::later_turn_fails(
        vec![span("r1", "", SpanStatus::Ok), span("r2", "", SpanStatus::Error)],
        false
    )]
    #[case::every_turn_ok(
        vec![span("r1", "", SpanStatus::Ok), span("r2", "", SpanStatus::Unset)],
        true
    )]
    fn reads_every_root_status(#[case] spans: Vec<EvalSpan>, #[case] expected: bool) {
        assert_eq!(task_completed(&spans), expected);
    }
}
