mod codex;

use opentelemetry_proto::tonic::{
    collector::{logs::v1::ExportLogsServiceRequest, trace::v1::ExportTraceServiceRequest},
    common::v1::{KeyValue, any_value::Value},
    logs::v1::LogRecord,
    resource::v1::Resource,
    trace::v1::{ResourceSpans, ScopeSpans, Span, Status},
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, mem::size_of};

use super::{
    DecodeLimits, DecodedLogBatch, DecodedLogRecord, LogValue, Shared, attributes, limits::Budget,
    span,
};
use crate::{
    Error,
    normalize::{CLAUDE_CODE_EVENTS_SCOPE, visible_claude_response},
};

fn value<'a>(attributes: &'a [KeyValue], key: &str) -> Option<&'a Value> {
    attributes
        .iter()
        .rev()
        .find(|entry| entry.key == key)
        .and_then(|entry| entry.value.as_ref())
        .and_then(|value| value.value.as_ref())
}

fn text<'a>(attributes: &'a [KeyValue], key: &str) -> &'a str {
    match value(attributes, key) {
        Some(Value::StringValue(text)) => text,
        _ => "",
    }
}

fn effective_timestamp_ns(
    timestamp_ns: u64,
    observed_timestamp_ns: u64,
    attributes: &BTreeMap<String, LogValue>,
) -> u64 {
    if timestamp_ns != 0 {
        return timestamp_ns;
    }
    let event_timestamp = match attributes.get("event.timestamp") {
        Some(LogValue::String(value)) => {
            time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
                .ok()
                .and_then(|timestamp| u64::try_from(timestamp.unix_timestamp_nanos()).ok())
                .filter(|timestamp| *timestamp <= i64::MAX as u64)
        }
        _ => None,
    };
    event_timestamp.unwrap_or(observed_timestamp_ns)
}

fn absent_id(id: &[u8], length: usize) -> bool {
    id.is_empty() || (id.len() == length && id.iter().all(|byte| *byte == 0))
}

fn valid_optional_id(id: &[u8], length: usize) -> bool {
    id.is_empty() || id.len() == length
}

fn validate_record(record: &LogRecord) -> Result<(), Error> {
    if !valid_optional_id(&record.trace_id, 16)
        || !valid_optional_id(&record.span_id, 8)
        || record.time_unix_nano > i64::MAX as u64
        || record.observed_time_unix_nano > i64::MAX as u64
    {
        return Err(Error::InvalidPayload);
    }
    Ok(())
}

struct LogContext {
    missing_trace: bool,
    missing_parent: bool,
}

fn message(record: LogRecord) -> (Span, LogContext) {
    let timestamp = if record.time_unix_nano == 0 {
        record.observed_time_unix_nano
    } else {
        record.time_unix_nano
    };
    let missing_trace = absent_id(&record.trace_id, 16);
    let missing_parent = absent_id(&record.span_id, 8);
    let mut hash = Sha256::new();
    hash.update(b"litellm.claude.message.v1\0");
    if !missing_trace {
        hash.update(&record.trace_id);
    }
    if !missing_parent {
        hash.update(&record.span_id);
    }
    hash.update(text(&record.attributes, "event.name"));
    let uuid = text(&record.attributes, "message.uuid");
    if uuid.is_empty() {
        hash.update(timestamp.to_be_bytes());
        match value(&record.attributes, "event.sequence") {
            Some(Value::IntValue(sequence)) => hash.update(sequence.to_string()),
            _ => hash.update(text(&record.attributes, "event.sequence")),
        }
        hash.update(text(&record.attributes, "response"));
    } else {
        hash.update(uuid);
    }
    if missing_trace {
        hash.update(b"\0unassigned\0");
        hash.update(text(&record.attributes, "session.id"));
    }
    let identity = hash.finalize();
    let trace_id = if missing_trace {
        let session = text(&record.attributes, "session.id");
        if session.is_empty() {
            identity[..16].to_vec()
        } else {
            span::session_trace_id(session)
        }
    } else {
        record.trace_id
    };
    let failed = match value(&record.attributes, "success") {
        Some(Value::BoolValue(success)) => !success,
        Some(Value::StringValue(success)) => success == "false",
        _ => false,
    };
    (
        Span {
            trace_id,
            span_id: identity[..8].to_vec(),
            parent_span_id: if missing_trace || missing_parent {
                Vec::new()
            } else {
                record.span_id
            },
            name: format!("claude_code.{}", text(&record.attributes, "event.name")),
            kind: 1,
            start_time_unix_nano: timestamp,
            end_time_unix_nano: timestamp,
            status: (text(&record.attributes, "event.name") == "tool_result" && failed).then(
                || Status {
                    code: 2,
                    message: text(&record.attributes, "error").to_owned(),
                },
            ),
            attributes: record.attributes,
            ..Span::default()
        },
        LogContext {
            missing_trace,
            missing_parent,
        },
    )
}

fn duration_ns(attributes: &[KeyValue]) -> u64 {
    let ms = match value(attributes, "duration_ms") {
        Some(Value::IntValue(ms)) if *ms > 0 => *ms as f64,
        Some(Value::DoubleValue(ms)) if ms.is_finite() && *ms > 0.0 => *ms,
        Some(Value::StringValue(ms)) => ms
            .parse::<f64>()
            .ok()
            .filter(|val| val.is_finite() && *val > 0.0)
            .unwrap_or(0.0),
        _ => 0.0,
    };
    (ms * 1_000_000.0) as u64
}

fn is_codex_internal_title_prompt(prompt: &str) -> bool {
    prompt.starts_with(
        "You are a helpful assistant. You will be presented with a user prompt, and your job is to provide a short title",
    ) || prompt.starts_with("Generate a concise, single-line task title")
}

fn codex_message(record: LogRecord, timestamp: u64) -> (Span, LogContext) {
    let event = text(&record.attributes, "event.name");
    let missing_trace = absent_id(&record.trace_id, 16);
    let missing_parent = absent_id(&record.span_id, 8);
    let mut hash = Sha256::new();
    hash.update(b"litellm.codex.message.v1\0");
    if !missing_trace {
        hash.update(&record.trace_id);
    }
    if !missing_parent {
        hash.update(&record.span_id);
    }
    hash.update(event);
    hash.update(timestamp.to_be_bytes());
    for key in ["call_id", "item.id", "turn.id", "prompt", "response", "output"] {
        hash.update(text(&record.attributes, key));
    }
    let identity = hash.finalize();
    let trace_id = if missing_trace {
        let session = ["turn.id", "conversation.id", "session.id"]
            .into_iter()
            .map(|key| text(&record.attributes, key))
            .find(|val| !val.is_empty())
            .unwrap_or_default();
        if session.is_empty() {
            identity[..16].to_vec()
        } else {
            span::session_trace_id(session)
        }
    } else {
        record.trace_id
    };
    let root_id = span::codex_turn_root_span_id(&trace_id);
    let (span_id, parent_span_id) = if event == "codex.user_prompt" {
        (root_id, Vec::new())
    } else {
        (identity[..8].to_vec(), root_id)
    };
    let failed = match value(&record.attributes, "success") {
        Some(Value::BoolValue(success)) => !success,
        Some(Value::StringValue(success)) => success == "false",
        _ => false,
    };
    let start_ns = timestamp.saturating_sub(duration_ns(&record.attributes));
    (
        Span {
            trace_id,
            span_id,
            parent_span_id,
            name: event.to_owned(),
            kind: 1,
            start_time_unix_nano: start_ns,
            end_time_unix_nano: timestamp,
            status: failed.then(|| Status {
                code: 2,
                message: text(&record.attributes, "error").to_owned(),
            }),
            attributes: record.attributes,
            ..Span::default()
        },
        LogContext {
            missing_trace,
            missing_parent: false,
        },
    )
}

pub(super) fn decode(
    request: ExportLogsServiceRequest,
    limits: DecodeLimits,
) -> Result<DecodedLogBatch, Error> {
    let mut count = 0usize;
    let mut contexts = Vec::new();
    let mut resources = Vec::new();
    let mut decoded_records = Vec::new();
    let mut budget = Budget::with_bytes(
        limits,
        limits.decoded_log_bytes.min(limits.decoded_span_bytes),
    );
    for resource in request.resource_logs {
        let resource_attributes = resource
            .resource
            .as_ref()
            .map_or(&[][..], |resource| resource.attributes.as_slice());
        if resource_attributes.len() > limits.attributes {
            return Err(Error::TooLarge);
        }
        let resource_attributes = Shared::new(attributes::log_attributes(
            resource_attributes,
            &mut budget,
        )?);
        let resource_schema_url = resource.schema_url;
        let mut scopes = Vec::new();
        for scope in resource.scope_logs {
            let instrumentation_scope = scope.scope.unwrap_or_default();
            if instrumentation_scope.attributes.len() > limits.attributes {
                return Err(Error::TooLarge);
            }
            budget.consume(
                instrumentation_scope
                    .name
                    .len()
                    .checked_add(instrumentation_scope.version.len())
                    .and_then(|bytes| bytes.checked_add(size_of::<DecodedLogRecord>()))
                    .ok_or(Error::TooLarge)?,
            )?;
            let scope_name: Shared<String> = instrumentation_scope.name.clone().into();
            let scope_version: Shared<String> = instrumentation_scope.version.clone().into();
            let scope_attributes = Shared::new(attributes::log_attributes(
                &instrumentation_scope.attributes,
                &mut budget,
            )?);
            let scope_schema_url = scope.schema_url;
            count = count
                .checked_add(scope.log_records.len())
                .ok_or(Error::TooLarge)?;
            if count > limits.logs {
                return Err(Error::TooLarge);
            }
            let is_claude = instrumentation_scope.name == CLAUDE_CODE_EVENTS_SCOPE;
            let is_codex = instrumentation_scope.name == "codex_otel.log_only";
            let spans = scope
                .log_records
                .into_iter()
                .map(|record| {
                    validate_record(&record)?;
                    if record.attributes.len() > limits.attributes {
                        return Err(Error::TooLarge);
                    }
                    let typed_attributes =
                        attributes::log_attributes(&record.attributes, &mut budget)?;
                    let body = record
                        .body
                        .as_ref()
                        .map(|value| attributes::log_value(Some(value), &mut budget, 0))
                        .transpose()?;
                    let semantic_event_name = match typed_attributes.get("event.name") {
                        Some(LogValue::String(value)) => {
                            budget.consume(value.len())?;
                            Some(value.clone())
                        }
                        _ => None,
                    };
                    let effective_timestamp_ns = effective_timestamp_ns(
                        record.time_unix_nano,
                        record.observed_time_unix_nano,
                        &typed_attributes,
                    );
                    budget.consume(
                        record
                            .severity_text
                            .len()
                            .checked_add(record.event_name.len())
                            .and_then(|bytes| bytes.checked_add(record.trace_id.len()))
                            .and_then(|bytes| bytes.checked_add(record.span_id.len()))
                            .and_then(|bytes| bytes.checked_add(resource_schema_url.len()))
                            .and_then(|bytes| bytes.checked_add(scope_schema_url.len()))
                            .and_then(|bytes| bytes.checked_add(size_of::<DecodedLogRecord>()))
                            .ok_or(Error::TooLarge)?,
                    )?;
                    let mut decoded_record = DecodedLogRecord {
                        timestamp_ns: record.time_unix_nano,
                        effective_timestamp_ns,
                        observed_timestamp_ns: record.observed_time_unix_nano,
                        severity_number: record.severity_number,
                        severity_text: record.severity_text.clone(),
                        event_name: record.event_name.clone(),
                        semantic_event_name,
                        body,
                        attributes: typed_attributes,
                        resource_attributes: budget.clone_shared(&resource_attributes, map_size)?,
                        scope_name: budget.clone_shared(&scope_name, String::len)?,
                        scope_version: budget.clone_shared(&scope_version, String::len)?,
                        scope_attributes: budget.clone_shared(&scope_attributes, map_size)?,
                        resource_schema_url: resource_schema_url.clone(),
                        scope_schema_url: scope_schema_url.clone(),
                        trace_id: record.trace_id.clone(),
                        span_id: record.span_id.clone(),
                        trace_flags: record.flags,
                        dropped_attributes_count: record.dropped_attributes_count,
                        codex: None,
                    };
                    decoded_record.codex = codex::project(&decoded_record);
                    decoded_records.push(decoded_record);
                    let event = text(&record.attributes, "event.name");
                    if is_codex {
                        let keep = match event {
                            "codex.user_prompt" => {
                                !is_codex_internal_title_prompt(text(&record.attributes, "prompt"))
                            }
                            "codex.agent_response" | "codex.tool_result" => true,
                            _ => false,
                        };
                        if !keep {
                            return Ok(None);
                        }
                        let (span, context) = codex_message(record, effective_timestamp_ns);
                        contexts.push(context);
                        return Ok(Some(span));
                    }
                    if !is_claude
                        || !(matches!(event, "tool_result" | "compaction")
                            || (matches!(event, "assistant_response" | "api_request_body")
                                && visible_claude_response(
                                    "assistant_response",
                                    text(&record.attributes, "query_source"),
                                )))
                    {
                        return Ok(None);
                    }
                    let (span, context) = message(record);
                    contexts.push(context);
                    Ok(Some(span))
                })
                .collect::<Result<Vec<_>, Error>>()?
                .into_iter()
                .flatten()
                .collect();
            scopes.push(ScopeSpans {
                scope: Some(instrumentation_scope),
                spans,
                schema_url: scope_schema_url,
            });
        }
        resources.push(ResourceSpans {
            resource: resource.resource.or_else(|| Some(Resource::default())),
            scope_spans: scopes,
            schema_url: resource_schema_url,
        });
    }
    let (mut spans, mut span_budget) = span::flatten_with_existing_budget(
        ExportTraceServiceRequest {
            resource_spans: resources,
        },
        budget,
    )?;
    span_budget.consume(contexts.len() * size_of::<LogContext>())?;
    for (span, context) in spans.iter_mut().zip(contexts) {
        if context.missing_trace {
            span.attributes.remove("lens.original_trace_id");
        }
        if context.missing_trace || context.missing_parent {
            let key = "lens.capture.warning";
            let warning = "This native log has incomplete trace context. Its content is retained, but its execution parent is unconfirmed.";
            span_budget.consume(key.len() + warning.len() + 96)?;
            span.attributes.insert(key.to_owned(), warning.to_owned());
        }
    }
    Ok(DecodedLogBatch {
        records: decoded_records,
        claude_spans: spans,
    })
}

fn map_size(attributes: &BTreeMap<String, LogValue>) -> usize {
    attributes
        .iter()
        .map(|(key, value)| key.len().saturating_add(log_value_size(value)))
        .sum()
}

fn log_value_size(value: &LogValue) -> usize {
    match value {
        LogValue::Null | LogValue::Boolean(_) | LogValue::Integer(_) | LogValue::Double(_) => {
            size_of::<LogValue>()
        }
        LogValue::String(value) => size_of::<LogValue>().saturating_add(value.len()),
        LogValue::Bytes(value) => size_of::<LogValue>().saturating_add(value.len()),
        LogValue::Array(values) => values
            .iter()
            .map(log_value_size)
            .fold(size_of::<LogValue>(), usize::saturating_add),
        LogValue::Map(values) => size_of::<LogValue>().saturating_add(map_size(values)),
    }
}
