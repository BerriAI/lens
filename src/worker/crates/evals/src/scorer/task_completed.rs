use std::collections::BTreeSet;

use super::{EvalSpan, SpanStatus};

pub fn root(spans: &[EvalSpan]) -> Option<&EvalSpan> {
    let ids: BTreeSet<&str> = spans.iter().map(|span| span.span_id.as_str()).collect();
    spans
        .iter()
        .filter(|span| {
            span.parent_span_id.is_empty()
                || span.parent_span_id == span.span_id
                || !ids.contains(span.parent_span_id.as_str())
        })
        .min_by(|left, right| (left.start_ns, &left.span_id).cmp(&(right.start_ns, &right.span_id)))
}

pub fn task_completed(spans: &[EvalSpan]) -> bool {
    root(spans).is_some_and(|span| span.status != SpanStatus::Error)
}
