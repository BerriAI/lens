use super::EvalSpan;

pub fn called_before(spans: &[EvalSpan], first: &str, then: &str) -> bool {
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
