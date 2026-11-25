use std::sync::Arc;

use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::Utc;
use futures_util::{StreamExt, stream};
use lens_auth::SessionRepository;
use litellm_traces::request::{
    CONVERSATION_PAGE_SIZE_DEFAULT, CONVERSATION_PAGE_SIZE_MAX, TRACE_AGENT_NAME_MAX_LENGTH,
    TRACE_PAGE_SIZE_MAX, TraceConversationRequest, TraceDetailRequest, TraceSpanRequest,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use super::{App, TraceReader, scope};
use crate::{
    auth,
    error::{McpToolError as ToolError, TraceHttpError},
};

const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const BATCH_MAX: usize = 16;
const HOUR_MS: i64 = 3_600_000;
const DEFAULT_HOURS: u32 = 24;
const TRACE_PAGE_SIZE_DEFAULT: u16 = 50;
const INSTRUCTIONS: &str = "Read only access to the agent traces this credential may see in Lens. \
Start with list_traces to find a run, then get_trace for its steps and cost, \
get_trace_conversation for the session's earlier turns and get_span for one step's full input and output.";

#[derive(Deserialize)]
struct Call {
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Deserialize)]
pub struct ToolCall {
    pub name: String,
    #[serde(default)]
    pub arguments: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListAgents {
    #[serde(default)]
    hours: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListTraces {
    #[serde(default)]
    agent: Option<String>,
    #[serde(default)]
    hours: Option<u32>,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    include_internal: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetTrace {
    trace_id: String,
    #[serde(default)]
    trace_ref: String,
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    page_size: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GetSpan {
    trace_id: String,
    span_id: String,
    #[serde(default)]
    trace_ref: String,
}

pub fn tools() -> Value {
    let hours = json!({"type":"integer","minimum":1,"description":"How many hours back to look. Defaults to 24 and is capped at the Lens retention window."});
    let cursor = json!({"type":"string","maxLength":512,"description":"next_cursor from the previous page."});
    let trace_id = json!({"type":"string","description":"trace_id from list_traces."});
    let trace_ref = json!({"type":"string","description":"trace_ref from list_traces. Pass it when present so the same run is read."});
    json!([
        {
            "name":"list_agents",
            "description":"List the agents that sent traces to Lens, with run and failure counts.",
            "inputSchema":{"type":"object","properties":{"hours":hours},"additionalProperties":false},
            "annotations":{"readOnlyHint":true}
        },
        {
            "name":"list_traces",
            "description":"List recent traces, newest first, with input preview, status, models, token counts and cost (spend, in USD, when the gateway priced the calls).",
            "inputSchema":{"type":"object","properties":{
                "agent":{"type":"string","maxLength":TRACE_AGENT_NAME_MAX_LENGTH,"description":"Only traces from this agent name, as returned by list_agents."},
                "hours":hours,
                "cursor":cursor,
                "include_internal":{"type":"boolean","description":"Also return traces of Lens's own analysis calls."}
            },"additionalProperties":false},
            "annotations":{"readOnlyHint":true}
        },
        {
            "name":"get_trace",
            "description":"Open one trace: its summary with total cost, its agents and its spans (LLM calls, tool calls and errors) in order.",
            "inputSchema":{"type":"object","properties":{
                "trace_id":trace_id,
                "trace_ref":trace_ref,
                "cursor":cursor,
                "page_size":{"type":"integer","minimum":1,"maximum":TRACE_PAGE_SIZE_MAX,"description":"Spans per page. Defaults to 50."}
            },"required":["trace_id"],"additionalProperties":false},
            "annotations":{"readOnlyHint":true}
        },
        {
            "name":"get_trace_conversation",
            "description":"Read the earlier turns of the session a trace belongs to: each previous user input with the agent's answer, oldest first. The trace's own input and output are in get_trace and get_span.",
            "inputSchema":{"type":"object","properties":{
                "trace_id":trace_id,
                "trace_ref":trace_ref,
                "cursor":cursor,
                "page_size":{"type":"integer","minimum":1,"maximum":CONVERSATION_PAGE_SIZE_MAX,"description":"Turns per page. Defaults to 10."}
            },"required":["trace_id"],"additionalProperties":false},
            "annotations":{"readOnlyHint":true}
        },
        {
            "name":"get_span",
            "description":"Read one span of a trace with its full input, output and attributes. Its cost is on the span in get_trace.",
            "inputSchema":{"type":"object","properties":{
                "trace_id":trace_id,
                "span_id":{"type":"string","description":"span_id from get_trace."},
                "trace_ref":trace_ref
            },"required":["trace_id","span_id"],"additionalProperties":false},
            "annotations":{"readOnlyHint":true}
        }
    ])
}

fn arguments<T: DeserializeOwned>(value: Option<Value>) -> Result<T, ToolError> {
    Ok(serde_json::from_value(value.unwrap_or_else(|| json!({})))?)
}

fn page_size(value: Option<u16>, default: u16, max: u16) -> Result<Option<u16>, ToolError> {
    let size = value.unwrap_or(default);
    if (1..=max).contains(&size) {
        Ok(Some(size))
    } else {
        Err(ToolError::OutOfRange {
            argument: "page_size",
            max: usize::from(max),
        })
    }
}

fn text<T: Serialize>(value: &T) -> Result<String, ToolError> {
    Ok(serde_json::to_string(value)?)
}

pub async fn call<B: TraceReader>(
    reader: &B,
    config: super::TraceConfig,
    scope: litellm_traces::query::named::ReadAccessParams,
    tool: ToolCall,
) -> Result<String, ToolError> {
    let now = Utc::now().timestamp_millis();
    let window = |hours: Option<u32>| {
        let hours = i64::from(hours.unwrap_or(DEFAULT_HOURS).max(1))
            .min(config.retention_days.max(1).saturating_mul(24));
        now.saturating_sub(hours.saturating_mul(HOUR_MS))
    };
    match tool.name.as_str() {
        "list_agents" => {
            let request: ListAgents = arguments(tool.arguments)?;
            let agents = reader
                .agents(scope, window(request.hours), now, config.agent_limit)
                .await?;
            text(&super::TraceAgentList { agents })
        }
        "list_traces" => {
            let request: ListTraces = arguments(tool.arguments)?;
            if request
                .agent
                .as_ref()
                .is_some_and(|agent| agent.chars().count() > TRACE_AGENT_NAME_MAX_LENGTH)
            {
                return Err(ToolError::OutOfRange {
                    argument: "agent",
                    max: TRACE_AGENT_NAME_MAX_LENGTH,
                });
            }
            text(
                &reader
                    .list(
                        scope,
                        request.agent.unwrap_or_default(),
                        request.include_internal,
                        false,
                        window(request.hours),
                        now,
                        request.cursor,
                        config.list_limit,
                    )
                    .await?,
            )
        }
        "get_trace" => {
            let request: GetTrace = arguments(tool.arguments)?;
            let detail = TraceDetailRequest {
                trace_ref: request.trace_ref,
                cursor: request.cursor,
                page_size: page_size(
                    request.page_size,
                    TRACE_PAGE_SIZE_DEFAULT,
                    TRACE_PAGE_SIZE_MAX,
                )?,
            };
            text(
                &reader
                    .trace(scope, request.trace_id, detail)
                    .await?
                    .ok_or(ToolError::Missing { resource: "Trace" })?,
            )
        }
        "get_trace_conversation" => {
            let request: GetTrace = arguments(tool.arguments)?;
            let conversation = TraceConversationRequest {
                trace_ref: request.trace_ref,
                cursor: request.cursor,
                page_size: page_size(
                    request.page_size,
                    CONVERSATION_PAGE_SIZE_DEFAULT,
                    CONVERSATION_PAGE_SIZE_MAX,
                )?,
            };
            text(
                &reader
                    .conversation(scope, request.trace_id, conversation)
                    .await?
                    .ok_or(ToolError::Missing { resource: "Trace" })?,
            )
        }
        "get_span" => {
            let request: GetSpan = arguments(tool.arguments)?;
            let span = TraceSpanRequest {
                trace_ref: request.trace_ref,
            };
            text(
                &reader
                    .span(scope, request.trace_id, request.span_id, span)
                    .await?
                    .ok_or(ToolError::Missing { resource: "Span" })?,
            )
        }
        _ => Err(ToolError::UnknownTool(tool.name)),
    }
}

fn failure(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

fn success(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}

fn initialize(params: &Value) -> Value {
    let requested = params.get("protocolVersion").and_then(Value::as_str);
    let version = PROTOCOL_VERSIONS
        .into_iter()
        .find(|version| Some(*version) == requested)
        .unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion":version,
        "capabilities":{"tools":{}},
        "serverInfo":{"name":"lens","title":"LiteLLM Lens","version":env!("CARGO_PKG_VERSION")},
        "instructions":INSTRUCTIONS
    })
}

async fn answer<R: SessionRepository, B: TraceReader>(
    app: &App<R, B>,
    scope: litellm_traces::query::named::ReadAccessParams,
    message: Value,
) -> Option<Value> {
    let Ok(call) = serde_json::from_value::<Call>(message) else {
        return Some(failure(Value::Null, -32600, "Invalid JSON-RPC request"));
    };
    let id = call.id?;
    Some(match call.method.as_str() {
        "initialize" => success(id, initialize(&call.params)),
        "ping" => success(id, json!({})),
        "tools/list" => success(id, json!({"tools":tools()})),
        "tools/call" => match serde_json::from_value::<ToolCall>(call.params) {
            Ok(tool) => {
                let (text, failed) = match self::call(&app.reader, app.config, scope, tool).await {
                    Ok(text) => (text, false),
                    Err(error) => (error.to_string(), true),
                };
                success(
                    id,
                    json!({"content":[{"type":"text","text":text}],"isError":failed}),
                )
            }
            Err(_) => failure(id, -32602, "tools/call needs a tool name"),
        },
        _ => failure(id, -32601, "Method not found"),
    })
}

fn respond(answer: Option<Value>) -> Response {
    answer.map_or_else(
        || StatusCode::ACCEPTED.into_response(),
        |answer| Json(answer).into_response(),
    )
}

pub(super) async fn handle<R: SessionRepository, B: TraceReader>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Response, TraceHttpError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let scope = scope(&identity)?;
    Ok(match serde_json::from_slice::<Value>(&body) {
        Ok(Value::Array(messages)) if (1..=BATCH_MAX).contains(&messages.len()) => {
            let answers: Vec<Value> = stream::iter(messages)
                .then(|message| answer(&app, scope.clone(), message))
                .filter_map(std::future::ready)
                .collect()
                .await;
            respond((!answers.is_empty()).then(|| Value::Array(answers)))
        }
        Ok(message @ Value::Object(_)) => respond(answer(&app, scope, message).await),
        _ => respond(Some(failure(
            Value::Null,
            -32600,
            "Invalid JSON-RPC request",
        ))),
    })
}
