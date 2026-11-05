use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};

use opentelemetry_proto::tonic::{
    collector::trace::v1::ExportTraceServiceRequest,
    common::v1::{KeyValue, any_value::Value},
    trace::v1::{ResourceSpans, ScopeSpans, Span, span::SpanKind, status::StatusCode},
};

use super::{
    DecodedEvent, DecodedSpan,
    attributes::attributes,
    limits::{Budget, DecodeLimits},
};
use crate::{
    Error, Shared,
    normalize::{CLAUDE_CODE_EVENTS_SCOPE, CLAUDE_CODE_SCOPE, SpanContext, normalize},
};

pub(super) fn flatten(
    request: ExportTraceServiceRequest,
    limits: DecodeLimits,
) -> Result<Vec<DecodedSpan>, Error> {
    flatten_with_budget(request, limits).map(|(spans, _)| spans)
}

pub(super) fn flatten_with_budget(
    request: ExportTraceServiceRequest,
    limits: DecodeLimits,
) -> Result<(Vec<DecodedSpan>, Budget), Error> {
    flatten_with_existing_budget(request, Budget::new(limits))
}

pub(super) fn flatten_with_existing_budget(
    request: ExportTraceServiceRequest,
    mut budget: Budget,
) -> Result<(Vec<DecodedSpan>, Budget), Error> {
    let mut spans = Vec::new();
    for resource in request.resource_spans {
        append_resource(resource, &mut budget, &mut spans)?;
    }
    Ok((spans, budget))
}

fn append_resource(
    resource: ResourceSpans,
    budget: &mut Budget,
    spans: &mut Vec<DecodedSpan>,
) -> Result<(), Error> {
    let attributes = Shared::new(attributes(
        resource
            .resource
            .map(|resource| resource.attributes)
            .unwrap_or_default(),
        budget,
    )?);
    for scope in resource.scope_spans {
        append_scope(scope, &attributes, budget, spans)?;
    }
    Ok(())
}

fn append_scope(
    mut scope_spans: ScopeSpans,
    resource: &Shared<BTreeMap<String, String>>,
    budget: &mut Budget,
    spans: &mut Vec<DecodedSpan>,
) -> Result<(), Error> {
    let scope = scope_spans.scope.unwrap_or_default();
    if scope.attributes.len() > budget.limits.attributes {
        return Err(Error::TooLarge);
    }
    budget.consume(scope.name.len() + scope.version.len())?;
    let scope_name: Shared<String> = scope.name.into();
    let scope_version: Shared<String> = scope.version.into();
    if matches!(scope_name.as_str(), "codex_cli_rs" | "codex-app-server") {
        preprocess_codex_spans(&mut scope_spans.spans);
    }
    for span in scope_spans.spans {
        if spans.len() >= budget.limits.spans {
            return Err(Error::TooLarge);
        }
        validate_span(&span, &budget.limits)?;
        budget.consume(
            span.name.len()
                + span.trace_state.len()
                + span
                    .status
                    .as_ref()
                    .map_or(0, |status| status.message.len())
                + size_of::<DecodedSpan>()
                + 128,
        )?;
        spans.push(decoded_span(
            span,
            resource,
            &scope_name,
            &scope_version,
            budget,
        )?);
    }
    Ok(())
}

fn raw_attr<'a>(span: &'a Span, key: &str) -> Option<&'a str> {
    span.attributes
        .iter()
        .rev()
        .find(|entry| entry.key == key)
        .and_then(|entry| entry.value.as_ref())
        .and_then(|value| match value.value.as_ref()? {
            Value::StringValue(text) => Some(text.as_str()),
            _ => None,
        })
}

fn clone_attr(span: &Span, key: &str) -> Option<KeyValue> {
    span.attributes
        .iter()
        .rev()
        .find(|entry| entry.key == key)
        .cloned()
}

fn has_attr(span: &Span, key: &str) -> bool {
    span.attributes.iter().any(|entry| entry.key == key)
}

fn preprocess_codex_spans(spans: &mut Vec<Span>) {
    let persist_traces: HashSet<Vec<u8>> = spans
        .iter()
        .filter(|span| span.name == "persist_rollout_items")
        .map(|span| span.trace_id.clone())
        .collect();
    let append_traces: HashSet<Vec<u8>> = spans
        .iter()
        .filter(|span| span.name == "append_items")
        .map(|span| span.trace_id.clone())
        .collect();
    let temporary_traces: HashSet<Vec<u8>> = spans
        .iter()
        .filter(|span| {
            span.name == "turn/start"
                && raw_attr(span, "rpc.request_id")
                    .is_some_and(|id| id.starts_with("temporary-structured-turn-"))
        })
        .map(|span| span.trace_id.clone())
        .chain(persist_traces.difference(&append_traces).cloned())
        .collect();
    let active_traces: HashSet<Vec<u8>> = spans
        .iter()
        .filter(|span| !temporary_traces.contains(&span.trace_id))
        .map(|span| span.trace_id.clone())
        .collect();
    for trace_id in &active_traces {
        let stream_req_parent: BTreeMap<Vec<u8>, Vec<u8>> = spans
            .iter()
            .filter(|span| &span.trace_id == trace_id && span.name == "responses.stream_request")
            .map(|span| (span.span_id.clone(), span.parent_span_id.clone()))
            .collect();
        let endpoint_by_model_call: BTreeMap<Vec<u8>, Vec<u8>> = spans
            .iter()
            .filter(|span| {
                &span.trace_id == trace_id && span.name == "endpoint_session.stream_encoded_json_with"
            })
            .filter_map(|span| {
                let model_call_id = stream_req_parent
                    .get(&span.parent_span_id)
                    .cloned()
                    .unwrap_or_else(|| span.parent_span_id.clone());
                (!model_call_id.is_empty()).then_some((model_call_id, span.span_id.clone()))
            })
            .collect();
        for span in spans.iter_mut() {
            if &span.trace_id == trace_id
                && span.name == "model_client.stream_responses_api"
                && let Some(endpoint_id) = endpoint_by_model_call.get(&span.span_id)
            {
                span.span_id = endpoint_id.clone();
            }
        }
        let mut usage_spans: Vec<(u64, usize, Option<KeyValue>, Option<KeyValue>)> = spans
            .iter()
            .enumerate()
            .filter(|(_, span)| &span.trace_id == trace_id && span.name == "handle_responses")
            .filter_map(|(index, span)| {
                let input = clone_attr(span, "gen_ai.usage.input_tokens");
                let output = clone_attr(span, "gen_ai.usage.output_tokens");
                (input.is_some() || output.is_some()).then_some((
                    span.start_time_unix_nano,
                    index,
                    input,
                    output,
                ))
            })
            .collect();
        usage_spans.sort_by_key(|(start, index, _, _)| (*start, *index));
        let mut model_calls: Vec<(u64, usize)> = spans
            .iter()
            .enumerate()
            .filter(|(_, span)| {
                &span.trace_id == trace_id && span.name == "model_client.stream_responses_api"
            })
            .map(|(index, span)| (span.start_time_unix_nano, index))
            .collect();
        model_calls.sort_by_key(|(start, index)| (*start, *index));
        let paired_count = model_calls.len().min(usage_spans.len());
        for ((_, call_index), (_, _, input, output)) in model_calls
            .into_iter()
            .zip(usage_spans.iter().cloned())
            .take(paired_count)
        {
            if let Some(input) = input
                && !has_attr(&spans[call_index], "gen_ai.usage.input_tokens")
            {
                spans[call_index].attributes.push(input);
            }
            if let Some(output) = output
                && !has_attr(&spans[call_index], "gen_ai.usage.output_tokens")
            {
                spans[call_index].attributes.push(output);
            }
        }
        if let Some(model_kv) = spans
            .iter()
            .filter(|span| {
                &span.trace_id == trace_id
                    && matches!(
                        span.name.as_str(),
                        "session_task.turn" | "turn/start" | "model_client.stream_responses_api"
                    )
            })
            .find_map(|span| clone_attr(span, "model"))
        {
            for span in spans.iter_mut() {
                if &span.trace_id == trace_id
                    && matches!(
                        span.name.as_str(),
                        "session_task.turn" | "turn/start" | "model_client.stream_responses_api"
                    )
                    && !has_attr(span, "model")
                {
                    span.attributes.push(model_kv.clone());
                }
            }
        }
    }
    spans.retain(|span| {
        let is_internal_tool = span.name == "dispatch_tool_call_with_state"
            || span.name.starts_with("unified_exec.")
            || span.name.starts_with("guardian_");
        !temporary_traces.contains(&span.trace_id)
            && !is_internal_tool
            && (matches!(
                span.name.as_str(),
                "session_task.turn"
                    | "turn/start"
                    | "model_client.stream_responses_api"
                    | "exec_command"
            ) || has_attr(span, "call_id")
                || has_attr(span, "tool_name")
                || [
                    "gen_ai.operation.name",
                    "gen_ai.request.model",
                    "gen_ai.input.messages",
                    "gen_ai.output.messages",
                ]
                .into_iter()
                .any(|key| has_attr(span, key)))
    });
    let root_turn_ids: BTreeMap<Vec<u8>, Vec<u8>> = spans
        .iter()
        .filter(|span| span.name == "turn/start")
        .map(|span| (span.trace_id.clone(), span.span_id.clone()))
        .collect();
    let task_turn_ids: BTreeMap<Vec<u8>, Vec<u8>> = spans
        .iter()
        .filter(|span| span.name == "session_task.turn")
        .map(|span| (span.trace_id.clone(), span.span_id.clone()))
        .collect();
    for span in spans.iter_mut() {
        let root_id = codex_turn_root_span_id(&span.trace_id);
        if span.name == "turn/start" {
            span.parent_span_id = root_id;
        } else if span.name == "session_task.turn" {
            if let Some(turn_start_id) = root_turn_ids.get(&span.trace_id) {
                span.parent_span_id = turn_start_id.clone();
            } else {
                span.parent_span_id = root_id;
            }
        } else if span.parent_span_id.is_empty() {
            span.parent_span_id = task_turn_ids
                .get(&span.trace_id)
                .or_else(|| root_turn_ids.get(&span.trace_id))
                .cloned()
                .unwrap_or(root_id);
        }
    }
}

pub(super) fn codex_turn_root_span_id(trace_id: &[u8]) -> Vec<u8> {
    let mut hash = Sha256::new();
    hash.update(b"litellm.codex.turn.root.v1\0");
    hash.update(trace_id);
    hash.finalize()[..8].to_vec()
}

fn valid_id(value: &[u8], length: usize) -> bool {
    value.len() == length && value.iter().any(|byte| *byte != 0)
}

fn validate_span(span: &Span, limits: &DecodeLimits) -> Result<(), Error> {
    if !valid_id(&span.trace_id, 16)
        || !valid_id(&span.span_id, 8)
        || (!span.parent_span_id.is_empty() && !valid_id(&span.parent_span_id, 8))
        || span.start_time_unix_nano > i64::MAX as u64
        || span.end_time_unix_nano > i64::MAX as u64
        || span.end_time_unix_nano < span.start_time_unix_nano
        || span
            .links
            .iter()
            .any(|link| !valid_id(&link.trace_id, 16) || !valid_id(&link.span_id, 8))
    {
        return Err(Error::InvalidPayload);
    }
    if span.events.len() > limits.events
        || span.links.len() > limits.links
        || span.attributes.len() > limits.attributes
        || span
            .links
            .iter()
            .any(|link| link.attributes.len() > limits.attributes)
        || span
            .events
            .iter()
            .any(|event| event.attributes.len() > limits.attributes)
    {
        return Err(Error::TooLarge);
    }
    Ok(())
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) fn session_trace_id(session: &str) -> Vec<u8> {
    Sha256::digest(format!("litellm.claude.session.v1\0{session}"))[..16].to_vec()
}

fn decoded_span(
    span: Span,
    resource_attributes: &Shared<BTreeMap<String, String>>,
    scope_name: &Shared<String>,
    scope_version: &Shared<String>,
    budget: &mut Budget,
) -> Result<DecodedSpan, Error> {
    let status = span.status.unwrap_or_default();
    let parent_span_id = hex_bytes(&span.parent_span_id);
    let mut span_attributes = attributes(span.attributes, budget)?;
    let original_trace_id = hex_bytes(&span.trace_id);
    let trace_id = if matches!(
        scope_name.as_str(),
        CLAUDE_CODE_SCOPE | CLAUDE_CODE_EVENTS_SCOPE
    ) && resource_attributes
        .get("lens.session.capture")
        .is_some_and(|value| value == "true")
        && let Some(session) = span_attributes
            .get("session.id")
            .filter(|value| !value.is_empty())
    {
        let trace_id = hex_bytes(&session_trace_id(session));
        let actor = span_attributes.get("agent_id").unwrap_or(session).clone();
        budget.consume(original_trace_id.len() + actor.len() + 256)?;
        span_attributes.insert("lens.original_trace_id".to_owned(), original_trace_id);
        span_attributes.insert("gen_ai.agent.id".to_owned(), actor);
        trace_id
    } else {
        original_trace_id
    };
    let events = span
        .events
        .into_iter()
        .map(|event| {
            budget.consume(event.name.len() + 96)?;
            Ok(DecodedEvent {
                name: event.name,
                attributes: attributes(event.attributes, budget)?,
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let normalization = normalize(&SpanContext {
        scope: scope_name.as_ref(),
        name: &span.name,
        parent_span_id: &parent_span_id,
        attributes: &span_attributes,
        events: &events,
        resource_attributes: resource_attributes.as_ref(),
    })?;
    let normalized = normalization.span;
    budget.consume(
        normalized.input.len()
            + normalized.output.len()
            + normalized.agent_name.as_ref().map_or(0, String::len)
            + normalized
                .framework
                .as_ref()
                .map_or(0, |integration| match integration {
                    crate::Integration::Other(name) => name.len(),
                    _ => 0,
                })
            + normalized.agent_metadata.byte_len()
            + normalized
                .calls
                .key_set()
                .into_iter()
                .flatten()
                .map(|key| match key {
                    crate::CallKey::LiteLlmRequest(id)
                    | crate::CallKey::ProviderResponse(id)
                    | crate::CallKey::ProviderRequest(id) => id.len() + size_of::<crate::CallKey>(),
                    crate::CallKey::Transport | crate::CallKey::GatewayAttempt => {
                        size_of::<crate::CallKey>()
                    }
                })
                .sum::<usize>()
            + normalized.model.as_ref().map_or(0, String::len)
            + normalization.display_name.as_ref().map_or(0, String::len),
    )?;
    Ok(DecodedSpan {
        trace_id,
        span_id: hex_bytes(&span.span_id),
        parent_span_id,
        trace_state: span.trace_state,
        name: normalization.display_name.unwrap_or(span.name),
        kind: SpanKind::try_from(span.kind)
            .unwrap_or(SpanKind::Unspecified)
            .as_str_name()
            .to_owned(),
        resource_attributes: budget.clone_shared(resource_attributes, |attributes| {
            attributes
                .iter()
                .map(|(key, value)| key.len() + value.len() + 96)
                .sum()
        })?,
        scope_name: budget.clone_shared(scope_name, String::len)?,
        scope_version: budget.clone_shared(scope_version, String::len)?,
        attributes: span_attributes,
        start_ns: span.start_time_unix_nano,
        end_ns: span.end_time_unix_nano,
        status_code: StatusCode::try_from(status.code)
            .unwrap_or(StatusCode::Unset)
            .as_str_name()
            .to_owned(),
        status_message: status.message,
        events,
        normalized,
        consumed_attributes: normalization.consumed_attributes,
    })
}
