use super::{Extraction, Integration, ObservationType, RoleEvidence, Rule, SpanContext, SpanFacts};
use crate::normalize::{
    OPENCODE_AGENT, OPENCODE_PROMPTS, OPENCODE_SCOPE, OPENCODE_TITLE_CALL, OPENCODE_TURN,
};

/// Set by the Vercel AI SDK on the model-call and tool spans of a run; OpenCode's other spans
/// under the same scope are its internals (`sql.execute`, `Plugin.load`), which are not agent work.
/// A root among them is still named after OpenCode, so its trace is not listed under the span name
const AI_SDK_OPERATION: &str = "ai.operationId";
/// The AI SDK's span around a provider call. The provider span inside it is the model call, so
/// counting both shows every call twice
const AI_SDK_CALL_WRAPPER: &str = "ai.streamText";

pub(super) struct Opencode;

impl Rule for Opencode {
    fn matches(&self, context: &SpanContext<'_>) -> bool {
        context.scope == OPENCODE_SCOPE
            && (is_agent_work(context) || context.parent_span_id.is_empty())
    }

    fn integration(&self, context: &SpanContext<'_>) -> Option<Integration> {
        is_agent_work(context).then_some(Integration::Opencode)
    }

    fn agent_name(&self, _: &SpanContext<'_>, recorded: Option<String>) -> Option<String> {
        recorded.or_else(|| Some(OPENCODE_AGENT.to_owned()))
    }

    fn adjust(&self, context: &SpanContext<'_>, extraction: Extraction) -> Extraction {
        if let Some(asked) = context.attributes.get(OPENCODE_TURN) {
            return extraction.map_facts(|facts| SpanFacts {
                role: Some(RoleEvidence::Declared(ObservationType::Agent)),
                input: asked.clone(),
                input_preview: Some(asked.clone()),
                ..facts
            });
        }
        let names_session = OPENCODE_PROMPTS.iter().any(|key| {
            context
                .attributes
                .get(*key)
                .is_some_and(|prompt| prompt.contains(OPENCODE_TITLE_CALL))
        });
        let wraps_call = context
            .attributes
            .get(AI_SDK_OPERATION)
            .is_some_and(|operation| operation == AI_SDK_CALL_WRAPPER);
        extraction.map_facts(|facts| SpanFacts {
            role: if wraps_call {
                Some(RoleEvidence::Declared(ObservationType::Chain))
            } else {
                facts.role
            },
            input_preview: if names_session {
                Some(String::new())
            } else {
                facts.input_preview
            },
            ..facts
        })
    }
}

fn is_agent_work(context: &SpanContext<'_>) -> bool {
    context.attributes.contains_key(AI_SDK_OPERATION)
        || context.attributes.contains_key(OPENCODE_TURN)
}

const EFFECT_SCOPE: &str = "effect";
const OPENCODE_SERVICE: &str = "opencode";
const STEP_ATTEMPT: &str = "SessionStep.attempt";
const TOOL_EXECUTE: &str = "Tool.execute";
const MODEL: &str = "gen_ai.request.model";
const TOOL_NAME: &str = "gen_ai.tool.name";

pub(super) struct OpencodeV2;

impl Rule for OpencodeV2 {
    fn matches(&self, context: &SpanContext<'_>) -> bool {
        context.scope == EFFECT_SCOPE
            && context
                .resource_attributes
                .get("service.name")
                .is_some_and(|service| service == OPENCODE_SERVICE)
    }

    fn integration(&self, _: &SpanContext<'_>) -> Option<Integration> {
        Some(Integration::Opencode)
    }

    fn adjust(&self, context: &SpanContext<'_>, extraction: Extraction) -> Extraction {
        let role = match context.name {
            STEP_ATTEMPT if context.attributes.contains_key(MODEL) => ObservationType::Llm,
            TOOL_EXECUTE if context.attributes.contains_key(TOOL_NAME) => ObservationType::Tool,
            _ => return extraction,
        };
        extraction.map_facts(|facts| SpanFacts {
            role: Some(RoleEvidence::Declared(role)),
            ..facts
        })
    }
}
