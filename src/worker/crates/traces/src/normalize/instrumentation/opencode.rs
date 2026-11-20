use super::{Extraction, Integration, Rule, SpanContext, SpanFacts};

const SCOPE: &str = "opencode";
const AGENT: &str = "opencode";
/// Set by the Vercel AI SDK on the model-call and tool spans of a run; OpenCode's other spans
/// under the same scope are its internals (`sql.execute`, `Plugin.load`), which are not agent work
const AI_SDK_OPERATION: &str = "ai.operationId";
const PROMPTS: [&str; 2] = ["ai.prompt", "ai.prompt.messages"];
/// How OpenCode opens the side call that names a session; it starts with the user's first turn,
/// so its prompt would otherwise be read as what the run was asked
const TITLE_CALL: &str = r#""role":"system","content":"You are a title generator."#;

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

    fn adjust(&self, context: &SpanContext<'_>, extraction: Extraction) -> Extraction {
        let names_session = PROMPTS.iter().any(|key| {
            context
                .attributes
                .get(*key)
                .is_some_and(|prompt| prompt.contains(TITLE_CALL))
        });
        if !names_session {
            return extraction;
        }
        extraction.map_facts(|facts| SpanFacts {
            input_preview: Some(String::new()),
            ..facts
        })
    }
}
