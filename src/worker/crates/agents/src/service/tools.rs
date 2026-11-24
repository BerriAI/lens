use futures_util::{StreamExt, TryStreamExt, stream};
use lens_server::tracing::{TraceConfig, TraceReader, mcp};
use litellm_traces::{QueryScope, request::TraceDetailRequest};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::Control;
use crate::{
    Error,
    conversations::{Access, Agent, BotInput},
    model::*,
};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Search {
    #[serde(default)]
    search: String,
    #[serde(default)]
    after: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Alias {
    alias: String,
    definition_id: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Batch {
    annotations: Vec<AnnotationInput>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Id {
    id: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Reports {
    trace: Option<TraceIdentity>,
    before: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Query {
    sql: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct BotState {
    id: String,
    enabled: bool,
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GatewayTool {
    name: String,
    arguments: Value,
}

fn tool<T: JsonSchema>(name: &str, description: &str, read_only: bool) -> Value {
    json!({"name":name,"description":description,"inputSchema":schemars::schema_for!(T),"annotations":{"readOnlyHint":read_only}})
}

pub(super) fn catalog() -> Value {
    let mut tools = mcp::tools().as_array().cloned().unwrap_or_default();
    tools.extend([
        tool::<Search>("list_definitions", "Search shared annotation definitions and aliases by meaning before creating a new one. Pages contain at most 100 definitions; continue with after=last definition ID.", true),
        tool::<DefinitionInput>("create_definition", "Create a shared annotation definition only when no existing meaning fits. Any agent may create one. Unique keys cannot silently change meaning. Returns the existing definition for an identical retry.", false),
        tool::<Alias>("alias_definition", "Consolidate an alternative key into an existing definition. Conflicting values from the same producer must be resolved first. The original IDs remain resolvable.", false),
        tool::<Batch>("annotate_traces", "Write up to 1000 typed annotations with evidence, atomically. One judgment per trace identity, producer and definition; reprocessing replaces your own judgment. Authorship is recorded by Lens. Unclassified traces are not successes.", false),
        tool::<AnnotationFilter>("query_annotations", "Find structured judgments across agents, optionally filtered by producer_id, definition_id/key, value, or trace identity. Returns annotations and next_cursor; pass next_cursor as after. Missing annotations mean no recorded classification.", true),
        tool::<ReportInput>("save_report", "Publish an evidence-backed investigation or other Markdown artifact linked to traces. Choose a stable ID to update your report without duplicates. Other agents can query it.", false),
        tool::<crate::artifacts::PublishArtifact>("publish_artifact", "Publish a file from your working directory, such as a chart, CSV, JSON, or Markdown file. Pass a relative path. PNG, JPEG and WebP appear inline in report posts; other files are downloadable. Use the returned ID in save_report.artifact_ids. File IDs are immutable and retries are safe.", false),
        tool::<Id>("get_artifact", "Read a report attachment by its ID, including media type and base64 file contents.", true),
        tool::<Id>("get_report", "Read a shared report and its trace evidence links.", true),
        tool::<Reports>("list_reports", "List report previews, optionally for one trace identity. Pages contain at most 50 reports. Continue with before set to the last updated_at, a colon, and id. Read full reports with get_report.", true),
        tool::<Query>("query_traces", "Run a SELECT-only analytical query over visible traces. Use trace_query_help first for schema and query constraints. The database enforces read-only access, scope and query limits.", true),
        tool::<Search>("trace_query_help", "Read the trace SQL schema and query instructions. Traces cannot be changed.", true),
        tool::<Search>("list_bots", "Find named persistent bots and their producer IDs. Use the ID when querying annotations produced by a specific bot.", true),
        tool::<BotInput>("create_bot", "Create a persistent Hermes bot with a standing task. It starts working and checks its ongoing task every ten minutes until paused; it can set its own native routines.", false),
        tool::<BotState>("set_bot_enabled", "Pause or resume a named bot's actual runtime, including its native scheduled work.", false),
        tool::<Search>("search_gateway_tools", "Discover connected tools through the configured LiteLLM MCP gateway. Search tool names and descriptions, then call a chosen tool by name.", true),
        tool::<Empty>("list_slack_threads", "List this agent's authorized Slack threads.", true),
        tool::<crate::slack::SlackMessage>("send_slack_message", "Send a message to a thread linked to this agent in Lens. Use a stable delivery ID. Uncertain sends are never automatically repeated.", false),
        tool::<GatewayTool>("call_gateway_tool", "Call a discovered LiteLLM MCP gateway tool with its documented arguments. Gateway credentials determine the allowed tools and upstream access.", false),
    ]);
    Value::Array(tools)
}

fn args<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, Error> {
    serde_json::from_value(value)
        .map_err(|_| Error::Invalid("Tool arguments do not match the schema"))
}

pub(super) async fn visible<B: TraceReader>(
    reader: &B,
    agent: &Agent,
    traces: &[TraceIdentity],
) -> Result<(), Error> {
    if traces.len() > 1000 {
        return Err(Error::Invalid(
            "Read or annotate at most 1000 traces per batch",
        ));
    }
    let scope = agent.scope.as_ref().ok_or(Error::Forbidden)?;
    stream::iter(traces.iter().cloned())
        .map(|trace| {
            let scope = scope.clone();
            async move {
                bounded_text(&trace.trace_id, 256)?;
                bounded_text(&trace.trace_ref, 2048)?;
                let value = reader
                    .trace(
                        scope.clone(),
                        trace.trace_id.clone(),
                        TraceDetailRequest {
                            trace_ref: trace.trace_ref.clone(),
                            cursor: None,
                            page_size: Some(1),
                        },
                    )
                    .await
                    .map_err(|_| Error::Runtime)?;
                value.ok_or(Error::NotFound).map(|_| ())
            }
        })
        .buffer_unordered(16)
        .try_collect::<Vec<_>>()
        .await?;
    Ok(())
}

pub(super) async fn call<B: TraceReader>(
    control: &Control,
    reader: &B,
    config: TraceConfig,
    agent: &Agent,
    name: &str,
    arguments: Value,
) -> Result<Value, Error> {
    let actor = Control::actor(agent);
    let scope = agent.scope.clone().ok_or(Error::Forbidden)?;
    let query_scope = if scope.all_teams {
        QueryScope::All
    } else {
        QueryScope::Owned {
            user_id: scope.user_id.clone(),
            team_ids: scope.team_ids.clone(),
        }
    };
    let access = Access {
        workspace: agent.workspace.clone(),
        owner: agent.owner.clone(),
        scope: scope.clone(),
        writable: true,
    };
    let store = control.store();
    let result = match name {
        "list_definitions" => {
            let input: Search = args(arguments)?;
            Ok(json!(
                store
                    .definitions(&actor.workspace, &input.search, &input.after)
                    .await?
            ))
        }
        "create_definition" => Ok(json!(
            store.create_definition(&actor, args(arguments)?).await?
        )),
        "alias_definition" => {
            let input: Alias = args(arguments)?;
            Ok(json!(
                store
                    .alias(&actor.workspace, &input.alias, &input.definition_id)
                    .await?
            ))
        }
        "annotate_traces" => {
            let input: Batch = args(arguments)?;
            visible(
                reader,
                agent,
                &input
                    .annotations
                    .iter()
                    .map(|a| a.trace.clone())
                    .collect::<Vec<_>>(),
            )
            .await?;
            Ok(json!({"written":store.annotate(&actor, &input.annotations).await?}))
        }
        "query_annotations" => Ok(json!(
            store
                .annotations(&actor.workspace, &args(arguments)?)
                .await?
        )),
        "save_report" => {
            let input: ReportInput = args(arguments)?;
            visible(reader, agent, &input.traces).await?;
            Ok(json!(store.save_report(&actor, &input).await?))
        }
        "publish_artifact" => control.publish_artifact(agent, args(arguments)?).await,
        "get_artifact" => {
            let input: Id = args(arguments)?;
            Ok(json!(store.artifact(&actor.workspace, &input.id).await?))
        }
        "get_report" => {
            let input: Id = args(arguments)?;
            Ok(json!(store.report(&actor.workspace, &input.id).await?))
        }
        "list_reports" => {
            let input: Reports = args(arguments)?;
            Ok(json!(
                store
                    .reports(
                        &actor.workspace,
                        input.trace.as_ref(),
                        input.before.as_deref()
                    )
                    .await?
            ))
        }
        "query_traces" => {
            let input: Query = args(arguments)?;
            reader.query(query_scope, input.sql).await.map_err(|_| Error::Invalid("Trace query was rejected. Read trace_query_help for the supported schema and query rules"))
        }
        "trace_query_help" => Ok(json!(
            reader.help(query_scope).await.map_err(|_| Error::Runtime)?
        )),
        "list_bots" => Ok(json!(
            store
                .agents(&access)
                .await?
                .into_iter()
                .filter(|a| matches!(a.runtime, crate::conversations::Runtime::Hermes))
                .collect::<Vec<_>>()
        )),
        "create_bot" => Ok(json!(control.create_bot(&access, &args(arguments)?).await?)),
        "set_bot_enabled" => {
            let input: BotState = args(arguments)?;
            if input.id == agent.id {
                return Err(Error::Invalid("Ask the main agent to pause this bot"));
            }
            Ok(json!(
                control.enabled(&access, &input.id, input.enabled).await?
            ))
        }
        "search_gateway_tools" => {
            let input: Search = args(arguments)?;
            let page = control
                .gateway_mcp(
                    "tools/list",
                    if input.after.is_empty() {
                        json!({})
                    } else {
                        json!({"cursor":input.after})
                    },
                )
                .await?;
            let query = input.search.to_lowercase();
            let tools = page["tools"]
                .as_array()
                .ok_or(Error::Runtime)?
                .iter()
                .filter(|tool| tool.to_string().to_lowercase().contains(&query))
                .cloned()
                .collect::<Vec<_>>();
            Ok(json!({"tools":tools,"next_cursor":page["nextCursor"]}))
        }
        "list_slack_threads" => control.store().slack_destinations(agent).await,
        "send_slack_message" => control.send_slack(agent, args(arguments)?).await,
        "call_gateway_tool" => {
            let input: GatewayTool = args(arguments)?;
            control
                .gateway_mcp(
                    "tools/call",
                    json!({"name":input.name,"arguments":input.arguments}),
                )
                .await
        }
        _ => {
            let result = mcp::call(
                reader,
                config,
                scope,
                mcp::ToolCall {
                    name: name.into(),
                    arguments: Some(arguments),
                },
            )
            .await
            .map_err(|_| {
                Error::Invalid("Trace tool was rejected. Check the tool name and trace identity")
            })?;
            Ok(serde_json::from_str(&result)?)
        }
    }?;
    Ok(trace_links(result, &control.0.ui_base))
}

fn trace_links(value: Value, ui_base: &str) -> Value {
    match value {
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(|value| trace_links(value, ui_base))
                .collect(),
        ),
        Value::Object(fields) => {
            let mut fields: serde_json::Map<String, Value> = fields
                .into_iter()
                .map(|(key, value)| (key, trace_links(value, ui_base)))
                .collect();
            if let (Some(id), Some(reference)) = (
                fields.get("trace_id").and_then(Value::as_str),
                fields.get("trace_ref").and_then(Value::as_str),
            ) {
                let query = url::form_urlencoded::Serializer::new(String::new())
                    .append_pair("tab", "traces")
                    .append_pair("trace", id)
                    .append_pair("trace_ref", reference)
                    .finish();
                fields.insert("lens_url".into(), json!(format!("{ui_base}?{query}")));
            }
            Value::Object(fields)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    fn trace_handoffs_preserve_the_full_scope_in_clickable_links() {
        let result = trace_links(
            json!({"annotations":[{"trace":{"trace_id":"a/b","trace_ref":"scope:+x&y"}}]}),
            "https://lens.example/ui/",
        );
        let url = url::Url::parse(
            result["annotations"][0]["trace"]["lens_url"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        let params: std::collections::HashMap<_, _> = url.query_pairs().collect();
        assert_eq!(url.path(), "/ui/");
        assert_eq!(params["tab"], "traces");
        assert_eq!(params["trace"], "a/b");
        assert_eq!(params["trace_ref"], "scope:+x&y");
    }
}
