use crate::wire;
use chrono::{DateTime, NaiveDateTime};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Serialize)]
pub struct Measurements {
    pub recorded_spans: usize,
    pub partial: bool,
    pub root_span_id: Option<String>,
    pub model_span_ids: Vec<String>,
    pub timing: Option<Timing>,
    pub usage: Usage,
    pub model_spans_without_input: Vec<String>,
    pub model_spans_without_output: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct Timing {
    pub root_seconds: f64,
    pub model_union_seconds: f64,
    pub outside_model_seconds: f64,
    pub before_first_model_seconds: Option<f64>,
    pub after_last_model_seconds: Option<f64>,
}

#[derive(Clone, Serialize)]
pub struct Usage {
    pub observed_model_calls: Option<usize>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub calls_with_input_tokens: usize,
    pub calls_with_output_tokens: usize,
    pub calls_with_recorded_cost: usize,
}

fn instant(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .and_then(|value| value.timestamp_nanos_opt())
        .or_else(|| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f")
                .ok()?
                .and_utc()
                .timestamp_nanos_opt()
        })
}

fn interval(part: &wire::TracePart) -> Option<(i64, i64)> {
    let start = instant(&part.start_time)?;
    let end = instant(&part.end_time)?;
    (end >= start).then_some((start, end))
}

fn rooted(part: &wire::TracePart, parts: &BTreeMap<&str, &wire::TracePart>) -> bool {
    let mut current = part;
    let mut seen = BTreeSet::new();
    while !current.parent_span_id.is_empty() {
        if !seen.insert(current.span_id.as_str()) {
            return false;
        }
        let Some(parent) = parts.get(current.parent_span_id.as_str()) else {
            return false;
        };
        current = parent;
    }
    true
}

fn timing(root: &wire::TracePart, models: &[&wire::TracePart]) -> Option<Timing> {
    let (start, end) = interval(root)?;
    let mut intervals = models
        .iter()
        .map(|part| {
            let (left, right) = interval(part)?;
            Some((left.clamp(start, end), right.clamp(start, end)))
        })
        .collect::<Option<Vec<_>>>()?;
    intervals.sort_unstable();
    let mut covered = 0_i128;
    let mut last = start;
    for (left, right) in &intervals {
        covered += i128::from(*right) - i128::from((*left).max(last).min(*right));
        last = last.max(*right);
    }
    let seconds = |nanos: i128| nanos as f64 / 1_000_000_000.0;
    let duration = i128::from(end) - i128::from(start);
    Some(Timing {
        root_seconds: seconds(duration),
        model_union_seconds: seconds(covered),
        outside_model_seconds: seconds(duration - covered),
        before_first_model_seconds: intervals
            .first()
            .map(|(left, _)| seconds(i128::from(*left) - i128::from(start))),
        after_last_model_seconds: intervals
            .last()
            .map(|_| seconds(i128::from(end) - i128::from(last))),
    })
}

pub(super) fn summarize(
    execution: &wire::Execution,
    parts: &[wire::TracePart],
    partial: bool,
) -> Measurements {
    let by_id: BTreeMap<_, _> = parts
        .iter()
        .map(|part| (part.span_id.as_str(), part))
        .collect();
    let roots: Vec<_> = parts
        .iter()
        .filter(|part| part.parent_span_id.is_empty())
        .collect();
    let complete = !partial
        && execution.root_seen
        && roots.len() == 1
        && parts.len() == by_id.len()
        && execution.span_count == parts.len() as i64
        && parts.iter().all(|part| rooted(part, &by_id));
    let models: Vec<_> = parts
        .iter()
        .filter(|part| {
            part.metrics
                .as_ref()
                .is_some_and(|metrics| metrics.model_call == Some(true))
        })
        .collect();
    let classified = parts.iter().all(|part| {
        part.metrics
            .as_ref()
            .is_some_and(|metrics| metrics.model_call.is_some())
    });
    let total = |field: fn(&wire::SpanMetrics) -> Option<u64>| {
        if !complete || !classified || models.is_empty() {
            return None;
        }
        models.iter().try_fold(0_u64, |sum, part| {
            sum.checked_add(field(part.metrics.as_ref()?)?)
        })
    };
    let missing = |field: fn(&wire::SpanMetrics) -> bool| {
        models
            .iter()
            .filter(|part| part.metrics.as_ref().is_some_and(|metrics| !field(metrics)))
            .map(|part| part.span_id.clone())
            .collect()
    };
    Measurements {
        recorded_spans: parts.len(),
        partial: !complete,
        root_span_id: (roots.len() == 1).then(|| roots[0].span_id.clone()),
        model_span_ids: models.iter().map(|part| part.span_id.clone()).collect(),
        timing: (complete && classified && parts.iter().all(|part| interval(part).is_some()))
            .then(|| timing(roots[0], &models))
            .flatten(),
        usage: Usage {
            observed_model_calls: classified.then_some(models.len()),
            input_tokens: total(|metrics| metrics.input_tokens),
            output_tokens: total(|metrics| metrics.output_tokens),
            cache_read_tokens: total(|metrics| metrics.cache_read_tokens),
            cache_write_tokens: total(|metrics| metrics.cache_write_tokens),
            calls_with_input_tokens: models
                .iter()
                .filter(|part| {
                    part.metrics
                        .as_ref()
                        .is_some_and(|m| m.input_tokens.is_some())
                })
                .count(),
            calls_with_output_tokens: models
                .iter()
                .filter(|part| {
                    part.metrics
                        .as_ref()
                        .is_some_and(|m| m.output_tokens.is_some())
                })
                .count(),
            calls_with_recorded_cost: models
                .iter()
                .filter(|part| {
                    part.metrics
                        .as_ref()
                        .is_some_and(|m| m.gateway_cost_usd.is_some())
                })
                .count(),
        },
        model_spans_without_input: missing(|metrics| metrics.input_present),
        model_spans_without_output: missing(|metrics| metrics.output_present),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::{fixture, rstest};
    use serde_json::json;

    #[fixture]
    fn execution() -> wire::Execution {
        serde_json::from_value(json!({"id":"run", "trace_id":"trace", "source":"traces", "team_id":"team", "name":"assistant", "start_time":"2026-01-01T00:00:00Z", "span_count":2, "root_seen":true})).unwrap()
    }

    fn part(id: &str, parent: &str, start: f64, end: f64, model: bool) -> wire::TracePart {
        let timestamp = |seconds: f64| {
            DateTime::from_timestamp(1_767_225_600, 0)
                .unwrap()
                .checked_add_signed(chrono::TimeDelta::nanoseconds(
                    (seconds * 1e9).round() as i64
                ))
                .unwrap()
                .to_rfc3339()
        };
        serde_json::from_value(json!({"execution_id":"run", "span_id":id, "parent_span_id":parent, "name":id, "kind":if model {"llm"} else {"agent"}, "content":"Input: hello\nOutput: Hi", "start_time":timestamp(start), "end_time":timestamp(end), "metrics":{"input_tokens":104847, "output_tokens":40, "cache_read_tokens":null, "cache_write_tokens":null, "input_present":true, "output_present":true, "model_call":model, "gateway_cost_usd":null}})).unwrap()
    }

    #[rstest]
    #[case::brief(98.626465, 1.0, 47.065685, 52.560780)]
    #[case::short(20.768177, 1.0, 4.598592, 17.169585)]
    #[case::admission(996.7, 987.2, 989.9, 994.0)]
    fn measures_wait_outside_recorded_model_intervals(
        execution: wire::Execution,
        #[case] end: f64,
        #[case] model_start: f64,
        #[case] model_end: f64,
        #[case] outside: f64,
    ) {
        let summary = summarize(
            &execution,
            &[
                part("root", "", 0.0, end, false),
                part("model", "root", model_start, model_end, true),
            ],
            false,
        );
        let timing = summary.timing.unwrap();
        assert!((timing.outside_model_seconds - outside).abs() < 1e-6);
        assert!((timing.before_first_model_seconds.unwrap() - model_start).abs() < 1e-6);
        assert!((timing.after_last_model_seconds.unwrap() - (end - model_end)).abs() < 1e-6);
        assert_eq!(summary.usage.input_tokens, Some(104847));
        assert_eq!(summary.usage.output_tokens, Some(40));
        assert_eq!(summary.usage.cache_read_tokens, None);
        assert_eq!(summary.model_span_ids, vec!["model"]);
    }

    #[rstest]
    fn union_clips_parallel_intervals_and_excludes_wrapper_usage(mut execution: wire::Execution) {
        execution.span_count = 4;
        let mut wrapper = part("wrapper", "root", 0.0, 12.0, false);
        wrapper.kind = "llm".into();
        let mut second = part("second", "wrapper", 4.0, 14.0, true);
        second.metrics.as_mut().unwrap().cache_read_tokens = Some(0);
        let mut first = part("first", "wrapper", -2.0, 6.0, true);
        first.metrics.as_mut().unwrap().cache_read_tokens = Some(0);
        let summary = summarize(
            &execution,
            &[part("root", "", 0.0, 12.0, false), wrapper, first, second],
            false,
        );
        let timing = summary.timing.unwrap();
        assert_eq!(timing.model_union_seconds, 12.0);
        assert_eq!(timing.outside_model_seconds, 0.0);
        assert_eq!(summary.usage.input_tokens, Some(209694));
        assert_eq!(summary.usage.cache_read_tokens, Some(0));
    }

    #[rstest]
    #[case::partial(true, true, 2, "root", "2026-01-01T00:00:02Z")]
    #[case::root_unseen(false, false, 2, "root", "2026-01-01T00:00:02Z")]
    #[case::missing_span(false, true, 3, "root", "2026-01-01T00:00:02Z")]
    #[case::orphan(false, true, 2, "absent", "2026-01-01T00:00:02Z")]
    #[case::cycle(false, true, 2, "model", "2026-01-01T00:00:02Z")]
    #[case::invalid_time(false, true, 2, "root", "invalid")]
    #[case::negative_time(false, true, 2, "root", "2025-01-01T00:00:00Z")]
    fn incomplete_or_invalid_recordings_have_no_timing(
        mut execution: wire::Execution,
        #[case] partial: bool,
        #[case] root_seen: bool,
        #[case] count: i64,
        #[case] parent: &str,
        #[case] end: &str,
    ) {
        execution.root_seen = root_seen;
        execution.span_count = count;
        let mut model = part("model", parent, 1.0, 2.0, true);
        model.end_time = end.into();
        assert!(
            summarize(
                &execution,
                &[part("root", "", 0.0, 3.0, false), model],
                partial
            )
            .timing
            .is_none()
        );
    }

    #[rstest]
    #[case::absent(None, None)]
    #[case::zero(Some(0), Some(0))]
    fn missing_messages_preserve_usage_and_unknown_counters(
        execution: wire::Execution,
        #[case] tokens: Option<u64>,
        #[case] expected: Option<u64>,
    ) {
        let mut model = part("model", "root", 1.0, 2.0, true);
        let metrics = model.metrics.as_mut().unwrap();
        metrics.input_tokens = tokens;
        metrics.input_present = false;
        metrics.output_present = false;
        let summary = summarize(
            &execution,
            &[part("root", "", 0.0, 3.0, false), model],
            false,
        );
        assert_eq!(summary.usage.input_tokens, expected);
        assert_eq!(summary.usage.output_tokens, Some(40));
        assert_eq!(
            summary.usage.calls_with_input_tokens,
            usize::from(tokens.is_some())
        );
        assert_eq!(summary.model_spans_without_input, vec!["model"]);
        assert_eq!(summary.model_spans_without_output, vec!["model"]);
    }
}
