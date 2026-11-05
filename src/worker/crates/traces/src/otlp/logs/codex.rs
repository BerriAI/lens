use super::super::{
    CodexContentState, CodexEventKind, CodexMessageRole, CodexProjection, DecodedLogRecord,
    LogValue,
};

fn string(attributes: &std::collections::BTreeMap<String, LogValue>, key: &str) -> Option<String> {
    match attributes.get(key) {
        Some(LogValue::String(value)) if !value.is_empty() => Some(value.clone()),
        _ => None,
    }
}

fn boolean(value: Option<&LogValue>) -> Option<bool> {
    match value {
        Some(LogValue::Boolean(value)) => Some(*value),
        Some(LogValue::String(value)) => match value.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn number(value: Option<&LogValue>) -> Option<f64> {
    match value {
        Some(LogValue::Integer(value)) => Some(*value as f64),
        Some(LogValue::Double(value)) if value.is_finite() => Some(*value),
        Some(LogValue::String(value)) => value.parse().ok(),
        _ => None,
    }
}

fn content(
    attributes: &std::collections::BTreeMap<String, LogValue>,
    key: &str,
) -> (CodexContentState, Option<String>) {
    match attributes.get(key) {
        Some(LogValue::String(value)) if key == "prompt" && value == "[REDACTED]" => {
            (CodexContentState::Redacted, None)
        }
        Some(LogValue::String(value)) => (CodexContentState::Present, Some(value.clone())),
        Some(_) => (CodexContentState::Malformed, None),
        None => (CodexContentState::Absent, None),
    }
}

pub(super) fn project(record: &DecodedLogRecord) -> Option<CodexProjection> {
    if record.scope_name.as_str() != "codex_otel.log_only"
        || string(&record.resource_attributes, "service.name").as_deref() != Some("codex_cli_rs")
        || string(&record.resource_attributes, "service.version").is_none()
        || string(&record.attributes, "app.version").is_none()
    {
        return None;
    }
    let event_name = record.semantic_event_name.as_deref()?;
    if !event_name.starts_with("codex.") {
        return None;
    }
    let (event_kind, message_role, content_key) = match event_name {
        "codex.conversation_starts" => (CodexEventKind::ConversationStarts, None, None),
        "codex.startup_phase" => (CodexEventKind::StartupPhase, None, None),
        "codex.user_prompt" => (
            CodexEventKind::UserPrompt,
            Some(CodexMessageRole::User),
            Some("prompt"),
        ),
        "codex.agent_response" => (
            CodexEventKind::AgentResponse,
            Some(CodexMessageRole::Assistant),
            Some("response"),
        ),
        "codex.tool_result" => (
            CodexEventKind::ToolResult,
            Some(CodexMessageRole::Tool),
            Some("output"),
        ),
        "codex.tool_decision" => (CodexEventKind::ToolDecision, None, None),
        "codex.sandbox_outcome" => (CodexEventKind::SandboxOutcome, None, None),
        "codex.api_request" => (CodexEventKind::ApiRequest, None, None),
        "codex.sse_event" => (CodexEventKind::SseEvent, None, None),
        "codex.turn_ttft" => (CodexEventKind::TurnTtft, None, None),
        "codex.turn_cost" => (CodexEventKind::TurnCost, None, None),
        "codex.skill_invocation" => (CodexEventKind::SkillInvocation, None, None),
        "codex.guardian_assessment" => (CodexEventKind::GuardianAssessment, None, None),
        event if event.starts_with("codex.websocket_") => (CodexEventKind::Websocket, None, None),
        event if event.starts_with("codex.plugin_install_") => (CodexEventKind::Plugin, None, None),
        _ => (CodexEventKind::Other, None, None),
    };
    let (content_state, content) = content_key
        .map(|key| content(&record.attributes, key))
        .map_or((None, None), |(state, content)| (Some(state), content));
    Some(CodexProjection {
        event_kind,
        message_role,
        content_state,
        content,
        conversation_id: string(&record.attributes, "conversation.id"),
        turn_id: string(&record.attributes, "turn.id"),
        item_id: string(&record.attributes, "item.id"),
        call_id: string(&record.attributes, "call_id"),
        tool_name: string(&record.attributes, "tool_name"),
        success: boolean(record.attributes.get("success")),
        duration_ms: number(record.attributes.get("duration_ms")),
        truncated: boolean(
            record
                .attributes
                .get(if event_name == "codex.agent_response" {
                    "response_truncated"
                } else {
                    "output_truncated"
                }),
        ),
    })
}
