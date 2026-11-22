use super::{Extraction, Integration, Rule, SpanContext, SpanFacts, attr, messages};

const MODEL_CALL_SCOPE: &str = "litellm";
const SCOPES: [&str; 2] = [MODEL_CALL_SCOPE, "litellm.lens.tool_steps"];

fn calling_agent(context: &SpanContext<'_>) -> Option<Integration> {
    match attr(context.attributes, "gen_ai.agent.name") {
        "claude-code" => Some(Integration::ClaudeCode),
        "codex" => Some(Integration::OpenaiCodex),
        _ => None,
    }
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
        extraction.map_facts(|facts| SpanFacts {
            input_preview: sent_task.or(facts.input_preview),
            ..facts
        })
    }
}
