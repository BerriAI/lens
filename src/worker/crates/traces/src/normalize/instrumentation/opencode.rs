use super::{
    CallEvidence, CallKey, Extraction, Integration, ObservationType, RoleEvidence, Rule,
    SpanContext, SpanFacts, present,
};
use crate::normalize::{
    OPENCODE_AGENT, OPENCODE_PROMPTS, OPENCODE_SCOPE, OPENCODE_TITLE_CALL, OPENCODE_TURN,
    OPENCODE_TURN_NAME, OpencodeV2Step, opencode_v2_step,
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

pub(super) struct OpencodeV2;

fn v2_step(context: &SpanContext<'_>) -> Option<OpencodeV2Step> {
    opencode_v2_step(
        context.scope,
        context.name,
        context.attributes,
        context.resource_attributes,
    )
}

impl Rule for OpencodeV2 {
    fn matches(&self, context: &SpanContext<'_>) -> bool {
        v2_step(context).is_some()
    }

    fn integration(&self, _: &SpanContext<'_>) -> Option<Integration> {
        Some(Integration::Opencode)
    }

    /// OpenCode 2 records its mode (`build`, `plan`) as the agent name, which would list one
    /// OpenCode install as several agents
    fn agent_name(&self, context: &SpanContext<'_>, _: Option<String>) -> Option<String> {
        present(context.resource_attributes, &["gen_ai.agent.name"])
            .or_else(|| Some(OPENCODE_AGENT.to_owned()))
    }

    fn adjust(&self, context: &SpanContext<'_>, extraction: Extraction) -> Extraction {
        let Some(step) = v2_step(context) else {
            return extraction;
        };
        let role = match step {
            OpencodeV2Step::Turn => ObservationType::Agent,
            OpencodeV2Step::ModelCall => ObservationType::Llm,
            OpencodeV2Step::Tool => ObservationType::Tool,
            OpencodeV2Step::Transport => ObservationType::Framework,
            OpencodeV2Step::Wrapper => ObservationType::Chain,
        };
        let adjusted = extraction.map_facts(|facts| SpanFacts {
            role: Some(RoleEvidence::Declared(role)),
            calls: if step == OpencodeV2Step::Transport {
                CallEvidence::complete(CallKey::Transport)
            } else {
                facts.calls
            },
            ..facts
        });
        if step == OpencodeV2Step::Turn {
            Extraction {
                display_name: Some(OPENCODE_TURN_NAME.to_owned()),
                ..adjusted
            }
        } else {
            adjusted
        }
    }
}
