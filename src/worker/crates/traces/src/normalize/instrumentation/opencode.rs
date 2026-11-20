use super::{Integration, Rule, SpanContext};

const SCOPE: &str = "opencode";
const AGENT: &str = "opencode";
/// Set by the Vercel AI SDK on the model-call and tool spans of a run; OpenCode's other spans
/// under the same scope are its internals (`sql.execute`, `Plugin.load`), which are not agent work
const AI_SDK_OPERATION: &str = "ai.operationId";

pub(super) struct Opencode;

impl Rule for Opencode {
    fn matches(&self, context: &SpanContext<'_>) -> bool {
        context.scope == SCOPE && context.attributes.contains_key(AI_SDK_OPERATION)
    }

    fn integration(&self, _: &SpanContext<'_>) -> Option<Integration> {
        Some(Integration::Opencode)
    }

    fn agent_name(&self, _: &SpanContext<'_>, recorded: Option<String>) -> Option<String> {
        recorded.or_else(|| Some(AGENT.to_owned()))
    }
}
