use super::EvalSpan;

pub(super) fn called_before(spans: &[EvalSpan], first: &str, then: &str) -> bool {
    let earliest = |tool: &str| {
        spans
            .iter()
            .filter(|span| span.tool_name.as_deref() == Some(tool))
            .map(|span| span.start_ns)
            .min()
    };
    match (earliest(first), earliest(then)) {
        (_, None) => true,
        (Some(first), Some(then)) => first < then,
        (None, Some(_)) => false,
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::SpanStatus;

    fn tool(id: &str, tool_name: &str, start_ns: i64) -> EvalSpan {
        EvalSpan {
            span_id: id.into(),
            parent_span_id: "root".into(),
            name: id.into(),
            start_ns,
            status: SpanStatus::Ok,
            tool_name: Some(tool_name.into()),
        }
    }

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
        vec![EvalSpan { tool_name: None, ..tool("search", "x", 1) }, tool("b", "write", 2)],
        false
    )]
    fn edges(#[case] spans: Vec<EvalSpan>, #[case] expected: bool) {
        assert_eq!(called_before(&spans, "search", "write"), expected);
    }
}
