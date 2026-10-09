use crate::cases::python_whitespace;
use crate::{Candidate, DatasetReader, Limits, ReadError, Scope, case_from_span, make_case};
use lens_contract::datasets::{
    BuildRequest, BuildResult, BuildSource, CaseSource, DatasetCase, DatasetMessage, DatasetRole,
    DatasetToolCall, FindingSource, SkipReason, SkippedCase, TextSource, TraceSource,
};
use litellm_traces::{ObservationType, UiContent};
use serde::Deserialize;
use std::collections::HashSet;

#[derive(Deserialize)]
struct TextLine {
    messages: Vec<DatasetMessage>,
    #[serde(default)]
    reply: String,
    #[serde(default)]
    tool_calls: Vec<DatasetToolCall>,
    #[serde(default)]
    expected: String,
    #[serde(default)]
    source: CaseSource,
    #[serde(default)]
    agent_version: String,
}

fn skipped(source: CaseSource, reason: SkipReason) -> Candidate {
    Candidate::Skipped(SkippedCase { source, reason })
}

async fn trace_cases(
    reader: &impl DatasetReader,
    source: &TraceSource,
    limits: Limits,
) -> Result<Vec<Candidate>, ReadError> {
    let origin = CaseSource {
        trace_id: source.trace_id.clone(),
        trace_ref: source.trace_ref.clone(),
        span_id: source.span_id.clone(),
        ..CaseSource::default()
    };
    if !source.span_id.is_empty() {
        let detail = reader
            .span(&source.trace_id, &source.span_id, &source.trace_ref)
            .await?;
        return Ok(vec![match detail {
            Some(detail) => case_from_span(&detail, origin, limits),
            None => skipped(origin, SkipReason::NoContent),
        }]);
    }
    let trace = reader.trace(&source.trace_id, &source.trace_ref).await?;
    let Some(trace) = trace else {
        return Ok(vec![skipped(origin, SkipReason::NoContent)]);
    };
    let mut spans: Vec<_> = trace
        .spans
        .iter()
        .filter(|span| span.kind == ObservationType::Llm)
        .collect();
    spans.sort_by(|left, right| {
        right
            .start_offset_ms
            .partial_cmp(&left.start_offset_ms)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for span in spans {
        if let Some(detail) = reader
            .span(&source.trace_id, &span.span_id, &source.trace_ref)
            .await?
            && matches!(detail.input_ui, UiContent::Messages { .. })
        {
            return Ok(vec![case_from_span(&detail, origin, limits)]);
        }
    }
    Ok(vec![skipped(origin, SkipReason::NoContent)])
}

fn parse_execution(value: &str) -> Option<(String, String)> {
    lens_contract::execution::ExecutionId::decode(value)
        .map(|identity| (identity.trace_id, identity.trace_ref))
}

async fn finding_cases(
    reader: &impl DatasetReader,
    source: &FindingSource,
    scope: &Scope,
    limits: Limits,
) -> Result<Vec<Candidate>, ReadError> {
    let findings = reader
        .findings(&source.lens_id, &source.finding_ids, scope)
        .await?;
    let mut seen = HashSet::new();
    let mut cases = Vec::new();
    for finding in findings {
        for evidence in finding.evidence {
            if !seen.insert(format!("{}\0{}", evidence.execution_id, evidence.span_id)) {
                continue;
            }
            let origin = CaseSource {
                span_id: evidence.span_id,
                finding_id: finding.id.clone(),
                lens_id: source.lens_id.clone(),
                ..CaseSource::default()
            };
            let Some((trace_id, trace_ref)) = parse_execution(&evidence.execution_id) else {
                cases.push(skipped(origin, SkipReason::NoContent));
                continue;
            };
            let located = CaseSource {
                trace_id,
                trace_ref,
                ..origin
            };
            let detail = reader
                .span(&located.trace_id, &located.span_id, &located.trace_ref)
                .await?;
            cases.push(match detail {
                Some(detail) => case_from_span(&detail, located, limits),
                None => skipped(located, SkipReason::NoContent),
            });
        }
    }
    Ok(cases)
}

fn text_cases(source: &TextSource, limits: Limits) -> Vec<Candidate> {
    let lines: Vec<_> = source
        .text
        .split([
            '\n', '\r', '\u{b}', '\u{c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}',
            '\u{2029}',
        ])
        .filter(|line| !line.trim_matches(python_whitespace).is_empty())
        .collect();
    if !lines.is_empty()
        && lines
            .iter()
            .all(|line| line.trim_start_matches(python_whitespace).starts_with('{'))
    {
        return lines
            .into_iter()
            .map(|line| {
                match jiter::JsonValue::parse(line.as_bytes(), true)
                    .ok()
                    .and_then(|parsed| serde_json::from_value::<TextLine>(text_value(&parsed)).ok())
                {
                    Some(parsed) => make_case(
                        parsed.messages,
                        parsed.reply,
                        parsed.tool_calls,
                        parsed.source,
                        parsed.expected,
                        parsed.agent_version,
                        limits,
                    ),
                    None => skipped(CaseSource::default(), SkipReason::Invalid),
                }
            })
            .collect();
    }
    vec![make_case(
        vec![DatasetMessage {
            role: DatasetRole::User,
            content: source.text.clone(),
            name: String::new(),
            tool_calls: Vec::new(),
        }],
        String::new(),
        Vec::new(),
        CaseSource::default(),
        String::new(),
        String::new(),
        limits,
    )]
}

fn text_value(value: &jiter::JsonValue<'_>) -> serde_json::Value {
    use jiter::JsonValue;
    use serde_json::Value;
    match value {
        // Text records have no numeric fields; numeric metadata stays ignorable without becoming text.
        JsonValue::Null | JsonValue::Int(_) | JsonValue::BigInt(_) | JsonValue::Float(_) => {
            Value::Null
        }
        JsonValue::Bool(value) => Value::Bool(*value),
        JsonValue::Str(value) => Value::String(value.to_string()),
        JsonValue::Array(values) => Value::Array(values.iter().map(text_value).collect()),
        JsonValue::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, value)| (key.to_string(), text_value(value)))
                .collect(),
        ),
    }
}

fn unread_source(source: &BuildSource) -> CaseSource {
    match source {
        BuildSource::Trace(source) => CaseSource {
            trace_id: source.trace_id.clone(),
            trace_ref: source.trace_ref.clone(),
            span_id: source.span_id.clone(),
            ..CaseSource::default()
        },
        BuildSource::Finding(source) => CaseSource {
            lens_id: source.lens_id.clone(),
            finding_id: source.finding_ids.first().cloned().unwrap_or_default(),
            ..CaseSource::default()
        },
        BuildSource::Text(_) => CaseSource::default(),
    }
}

pub async fn build_cases(
    request: &BuildRequest,
    reader: &impl DatasetReader,
    existing: &[DatasetCase],
    scope: &Scope,
    limits: Limits,
) -> Result<BuildResult, ReadError> {
    let mut seen: HashSet<_> = existing.iter().map(|case| case.id.clone()).collect();
    let mut result = BuildResult {
        cases: Vec::new(),
        skipped: Vec::new(),
    };
    for source in &request.sources {
        if existing.len() as i128 + result.cases.len() as i128 >= i128::from(limits.max_cases) {
            result.skipped.push(SkippedCase {
                source: unread_source(source),
                reason: SkipReason::OverLimit,
            });
            continue;
        }
        let candidates = match source {
            BuildSource::Trace(source) => trace_cases(reader, source, limits).await?,
            BuildSource::Finding(source) => finding_cases(reader, source, scope, limits).await?,
            BuildSource::Text(source) => text_cases(source, limits),
        };
        for candidate in candidates {
            let case = match candidate {
                Candidate::Skipped(skipped) => {
                    result.skipped.push(skipped);
                    continue;
                }
                Candidate::Case(case) => case,
            };
            let reason = if seen.contains(&case.id) {
                Some(SkipReason::Duplicate)
            } else if existing.len() as i128 + result.cases.len() as i128
                >= i128::from(limits.max_cases)
            {
                Some(SkipReason::OverLimit)
            } else {
                None
            };
            match reason {
                Some(reason) => result.skipped.push(SkippedCase {
                    source: case.source,
                    reason,
                }),
                None => {
                    seen.insert(case.id.clone());
                    result.cases.push(case);
                }
            }
        }
    }
    Ok(result)
}
