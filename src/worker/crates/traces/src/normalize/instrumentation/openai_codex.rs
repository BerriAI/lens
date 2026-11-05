use serde_json::json;

use super::{
    CallEvidence, CallKey, Extraction, Integration, ObservationType, RoleEvidence, Rule,
    SpanContext, SpanFacts, attr, present,
};

const SERVICE_NAME: &str = "codex_cli_rs";
const CODEX_APP_SERVER_SERVICE: &str = "codex-app-server";
const SERVICE_VERSION: &str = "0.166.0";
const CODEX_TRACES_SCOPE: &str = "codex_cli_rs";
const CODEX_APP_SERVER_SCOPE: &str = "codex-app-server";
const CODEX_LOGS_SCOPE: &str = "codex_otel.log_only";

pub(super) struct OpenaiCodex;

impl Rule for OpenaiCodex {
    fn matches(&self, context: &SpanContext<'_>) -> bool {
        if matches!(
            context.scope,
            CODEX_TRACES_SCOPE | CODEX_APP_SERVER_SCOPE | CODEX_LOGS_SCOPE
        ) {
            return true;
        }
        let is_codex_resource = context
            .resource_attributes
            .get("service.name")
            .is_some_and(|service| {
                matches!(
                    service.as_str(),
                    SERVICE_NAME | CODEX_APP_SERVER_SERVICE | "codex-cli"
                )
            })
            && (context
                .resource_attributes
                .get("service.version")
                .is_some_and(|version| version == SERVICE_VERSION)
                || context
                    .resource_attributes
                    .get("service.name")
                    .is_some_and(|service| {
                        matches!(service.as_str(), CODEX_APP_SERVER_SERVICE | "codex-cli")
                    }));
        is_codex_resource
            && ([
                "otel.name",
                "from",
                "tool_name",
                "codex.usage.reasoning_output_tokens",
                "codex.usage.total_tokens",
                "gen_ai.operation.name",
                "gen_ai.request.model",
                "gen_ai.usage.input_tokens",
                "gen_ai.usage.output_tokens",
            ]
            .into_iter()
            .any(|key| {
                context
                    .attributes
                    .get(key)
                    .is_some_and(|value| !value.is_empty())
            }) || context
                .events
                .iter()
                .any(|event| event.name.starts_with("codex.")))
    }

    fn integration(&self, _: &SpanContext<'_>) -> Option<Integration> {
        Some(Integration::OpenaiCodex)
    }

    fn agent_name(&self, context: &SpanContext<'_>, recorded: Option<String>) -> Option<String> {
        recorded
            .or_else(|| present(context.resource_attributes, &["gen_ai.agent.name"]))
            .or_else(|| present(context.attributes, &["gen_ai.agent.name"]))
            .or_else(|| {
                matches!(
                    context.scope,
                    CODEX_TRACES_SCOPE | CODEX_APP_SERVER_SCOPE | CODEX_LOGS_SCOPE
                )
                .then(|| "codex-cli".to_owned())
            })
    }

    fn adjust(&self, context: &SpanContext<'_>, extraction: Extraction) -> Extraction {
        let attributes = context.attributes;
        if context.scope == CODEX_LOGS_SCOPE {
            let event = match attr(attributes, "event.name") {
                "" => context.name,
                name => name,
            };
            let mut consumed = extraction.consumed_attributes;
            let mut display_name = extraction.display_name;
            let facts = match event {
                "codex.user_prompt" => {
                    let prompt = attr(attributes, "prompt");
                    let (input, input_preview) = if !prompt.is_empty() && prompt != "[REDACTED]" {
                        consumed.push("prompt");
                        (
                            json!([{"role": "user", "content": prompt}]).to_string(),
                            Some(prompt.chars().take(240).collect()),
                        )
                    } else {
                        (String::new(), None)
                    };
                    display_name = Some("turn/start".to_owned());
                    SpanFacts {
                        role: Some(RoleEvidence::Declared(ObservationType::Agent)),
                        model: present(attributes, &["model", "slug"]).or(extraction.facts.model),
                        input,
                        input_preview,
                        ..extraction.facts
                    }
                }
                "codex.agent_response" => {
                    let response = attr(attributes, "response");
                    let output = if !response.is_empty() {
                        consumed.push("response");
                        json!([{"role": "assistant", "content": response}]).to_string()
                    } else {
                        String::new()
                    };
                    SpanFacts {
                        role: Some(RoleEvidence::Declared(ObservationType::Chain)),
                        model: present(attributes, &["model", "slug"]).or(extraction.facts.model),
                        output,
                        ..extraction.facts
                    }
                }
                "codex.tool_result" => {
                    let arguments = attr(attributes, "arguments");
                    let output = attr(attributes, "output");
                    if !arguments.is_empty() {
                        consumed.push("arguments");
                    }
                    if !output.is_empty() {
                        consumed.push("output");
                    }
                    display_name = present(attributes, &["tool_name"]).or(display_name);
                    SpanFacts {
                        role: Some(RoleEvidence::Declared(ObservationType::Tool)),
                        input: arguments.to_owned(),
                        output: output.to_owned(),
                        tool_call_id: present(attributes, &["call_id"])
                            .or(extraction.facts.tool_call_id),
                        ..extraction.facts
                    }
                }
                _ => SpanFacts {
                    role: Some(RoleEvidence::Declared(ObservationType::Framework)),
                    ..extraction.facts
                },
            };
            return Extraction {
                facts,
                display_name,
                consumed_attributes: consumed,
            };
        }
        let is_codex_trace = matches!(context.scope, CODEX_TRACES_SCOPE | CODEX_APP_SERVER_SCOPE)
            || context.attributes.get("service.name").is_some_and(|s| s == "codex-cli")
            || context.resource_attributes.get("service.name").is_some_and(|s| {
                matches!(
                    s.as_str(),
                    SERVICE_NAME | CODEX_APP_SERVER_SERVICE | "codex-cli"
                )
            });
        if is_codex_trace {
            let facts = match context.name {
                "session_task.turn" | "turn/start" => SpanFacts {
                    role: Some(RoleEvidence::WrapperCandidate(ObservationType::Agent)),
                    model: present(attributes, &["model", "gen_ai.request.model"])
                        .or(extraction.facts.model),
                    ..extraction.facts
                },
                "model_client.stream_responses_api" => SpanFacts {
                    role: Some(RoleEvidence::Declared(ObservationType::Llm)),
                    model: present(attributes, &["model", "gen_ai.request.model"])
                        .or(extraction.facts.model),
                    calls: CallEvidence::complete(CallKey::Transport),
                    ..extraction.facts
                },
                _ => SpanFacts {
                    role: Some(RoleEvidence::Declared(ObservationType::Framework)),
                    ..extraction.facts
                },
            };
            return Extraction {
                facts,
                display_name: extraction.display_name,
                consumed_attributes: extraction.consumed_attributes,
            };
        }
        extraction
    }
}
