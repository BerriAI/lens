use super::{Extraction, Integration, Rule, SpanContext, SpanFacts, attr, messages};
use serde_json::Value;

const MODEL_CALL_SCOPE: &str = "litellm";
const SCOPES: [&str; 2] = [MODEL_CALL_SCOPE, "litellm.lens.tool_steps"];

fn calling_agent(context: &SpanContext<'_>) -> Option<Integration> {
    match attr(context.attributes, "gen_ai.agent.name") {
        "claude-code" => Some(Integration::ClaudeCode),
        "codex" => Some(Integration::OpenaiCodex),
        _ => None,
    }
}

fn text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n\n"),
        _ => String::new(),
    }
}

fn without_harness_blocks(text: &str) -> &str {
    let mut remaining = text.trim_start();
    while let Some((_, rest)) = remaining
        .strip_prefix('<')
        .and_then(|rest| rest.split_once('>'))
        .filter(|(tag, _)| !tag.is_empty() && !tag.starts_with('/'))
        .and_then(|(tag, rest)| rest.split_once(&format!("</{tag}>")))
    {
        remaining = rest.trim_start();
    }
    remaining
}

fn answers_a_tool(content: &Value) -> bool {
    content.as_array().is_some_and(|parts| {
        parts
            .iter()
            .any(|part| part.get("type").and_then(Value::as_str) == Some("tool_result"))
    })
}

fn typed_messages(input: &str) -> Vec<String> {
    serde_json::from_str::<Vec<Value>>(input)
        .unwrap_or_default()
        .iter()
        .filter(|item| item.get("role").and_then(Value::as_str) == Some("user"))
        .filter_map(|item| item.get("content"))
        .filter(|content| !answers_a_tool(content))
        .map(|content| without_harness_blocks(&text(content)).trim().to_owned())
        .filter(|typed| !typed.is_empty())
        .map(|typed| typed.chars().take(messages::PREVIEW_CHARS).collect())
        .collect()
}

pub(super) struct LitellmGateway;

impl Rule for LitellmGateway {
    fn matches(&self, context: &SpanContext<'_>) -> bool {
        SCOPES.contains(&context.scope)
    }

    fn integration(&self, context: &SpanContext<'_>) -> Option<Integration> {
        calling_agent(context)
    }

    fn adjust(&self, context: &SpanContext<'_>, extraction: Extraction) -> Extraction {
        let sent_task = Some(attr(context.attributes, "lens.task").trim())
            .filter(|task| context.scope == MODEL_CALL_SCOPE && !task.is_empty())
            .map(|task| task.chars().take(messages::PREVIEW_CHARS).collect());
        extraction.map_facts(|facts| {
            let mut typed = typed_messages(attr(context.attributes, "gen_ai.input.messages"));
            let follow_up = (typed.len() > 1).then(|| typed.pop()).flatten();
            SpanFacts {
                input_preview: follow_up
                    .or(sent_task)
                    .or(facts.input_preview)
                    .or_else(|| typed_messages(&facts.input).into_iter().next()),
                ..facts
            }
        })
    }
}
