use std::sync::Arc;

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::Utc;
use lens_auth::{
    McpError, SessionRepository,
    mcp::{self, McpRepository, StoredConnection},
};
use lens_contract::{
    auth::Identity,
    mcp::{McpAuthentication, McpCall, McpConnection, McpOutcome},
};
use litellm_traces::request::{
    CONVERSATION_PAGE_SIZE_DEFAULT, CONVERSATION_PAGE_SIZE_MAX, TRACE_AGENT_NAME_MAX_LENGTH,
    TRACE_PAGE_SIZE_MAX, TraceConversationRequest, TraceDetailRequest, TraceSpanRequest,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use super::{App, TraceReader, scope};
use crate::{
    auth,
    error::{McpHttpError, McpToolError as ToolError},
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
struct ToolCall {
    name: String,
    #[serde(default)]
    arguments: Option<Value>,
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

fn tools() -> Value {
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

async fn call<R: SessionRepository, B: TraceReader>(
    app: &App<R, B>,
    scope: litellm_traces::query::named::ReadAccessParams,
    tool: ToolCall,
) -> Result<String, ToolError> {
    let now = Utc::now().timestamp_millis();
    let window = |hours: Option<u32>| {
        let hours = i64::from(hours.unwrap_or(DEFAULT_HOURS).max(1))
            .min(app.config.retention_days.max(1).saturating_mul(24));
        now.saturating_sub(hours.saturating_mul(HOUR_MS))
    };
    match tool.name.as_str() {
        "list_agents" => {
            let request: ListAgents = arguments(tool.arguments)?;
            let agents = app
                .reader
                .agents(scope, window(request.hours), now, app.config.agent_limit)
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
                &app.reader
                    .list(
                        scope,
                        request.agent.unwrap_or_default(),
                        request.include_internal,
                        false,
                        window(request.hours),
                        now,
                        request.cursor,
                        app.config.list_limit,
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
                &app.reader
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
                &app.reader
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
                &app.reader
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
                let (text, failed) = match self::call(app, scope, tool).await {
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

pub(super) async fn handle<R: SessionRepository + McpRepository, B: TraceReader>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Response, McpHttpError> {
    handle_request(app, headers, method, body, None).await
}

pub(super) async fn handle_named<R: SessionRepository + McpRepository, B: TraceReader>(
    State(app): State<Arc<App<R, B>>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Response, McpHttpError> {
    handle_request(app, headers, method, body, Some(id)).await
}

fn metadata(
    connection: Option<&McpConnection>,
    identity: Option<&Identity>,
    message: &Value,
) -> McpCall {
    let method = message
        .get("method")
        .and_then(Value::as_str)
        .filter(|method| matches!(*method, "initialize" | "tools/list" | "tools/call" | "ping"))
        .unwrap_or("request");
    let tool = message
        .pointer("/params/name")
        .filter(|_| method == "tools/call")
        .and_then(Value::as_str)
        .and_then(|name| match name {
            "list_agents" | "list_traces" | "get_trace" | "get_trace_conversation" | "get_span" => {
                Some(name.to_owned())
            }
            _ => None,
        });
    McpCall {
        id: uuid::Uuid::new_v4().to_string(),
        at_ms: Utc::now().timestamp_millis(),
        connection_id: connection
            .map(|connection| connection.id.clone())
            .or_else(|| identity.map(mcp::legacy_id))
            .unwrap_or_default(),
        connection_name: connection
            .map(|connection| connection.name.clone())
            .unwrap_or_else(|| "Existing credential".into()),
        owner_id: connection
            .map(|connection| connection.owner_id.clone())
            .or_else(|| identity.and_then(|identity| identity.user_id.clone()))
            .unwrap_or_default(),
        owner_name: connection
            .map(|connection| connection.owner_name.clone())
            .unwrap_or_else(|| {
                identity
                    .filter(|identity| identity.token.is_some())
                    .and_then(|identity| {
                        identity
                            .user_name
                            .clone()
                            .or_else(|| identity.user_id.clone())
                    })
                    .unwrap_or_else(|| "Shared administrator credential".into())
            }),
        client: connection
            .map(|connection| connection.client.clone())
            .unwrap_or_else(|| "Not reported".into()),
        method: method.into(),
        tool,
        outcome: McpOutcome::Success,
        latency_ms: 0,
    }
}

async fn authenticate<R: SessionRepository + McpRepository, B>(
    app: &App<R, B>,
    headers: &HeaderMap,
    method: &Method,
    named: Option<&str>,
    message: &Value,
) -> Result<(Identity, Option<McpConnection>), McpHttpError> {
    let bearer = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
        .filter(|(scheme, value)| scheme.eq_ignore_ascii_case("bearer") && !value.is_empty())
        .map(|(_, value)| value);
    let token_id = bearer.and_then(mcp::token_id);
    if named.is_some()
        && token_id
            .as_deref()
            .is_some_and(|token_id| Some(token_id) != named)
    {
        return Err(McpError::Unauthorized.into());
    }
    let id = named.or(token_id.as_deref());
    let Some(id) = id else {
        return Ok((
            auth::identity(&app.authentication, headers, method).await?,
            None,
        ));
    };
    let id = uuid::Uuid::parse_str(id)
        .map_err(|_| McpError::NotFound)?
        .to_string();
    let records = app.authentication.sessions.mcp_connections().await?;
    let record = records
        .into_iter()
        .find(|record| record.connection.id == id)
        .ok_or(McpError::Unauthorized)?;
    let result = authenticate_record(app, headers, method, bearer, &record).await;
    if let Err(error) = &result {
        let mut call = metadata(Some(&record.connection), None, message);
        call.outcome = match error {
            McpHttpError::Connection(McpError::Expired) => McpOutcome::Expired,
            McpHttpError::Connection(McpError::Revoked) => McpOutcome::Revoked,
            McpHttpError::Connection(McpError::Forbidden) => McpOutcome::Forbidden,
            _ => McpOutcome::Unauthorized,
        };
        app.authentication.sessions.mcp_record(&call).await?;
    }
    Ok((result?, Some(record.connection)))
}

async fn authenticate_record<R: SessionRepository + McpRepository, B>(
    app: &App<R, B>,
    headers: &HeaderMap,
    method: &Method,
    bearer: Option<&str>,
    record: &StoredConnection,
) -> Result<Identity, McpHttpError> {
    let identity = match record.connection.authentication {
        McpAuthentication::Token => {
            mcp::check_token(record, bearer.ok_or(McpError::Unauthorized)?)?;
            lens_auth::local_admin()
        }
        McpAuthentication::Gateway => {
            let identity = auth::identity(&app.authentication, headers, method).await?;
            if identity.token.is_none()
                || identity.user_id.as_deref() != Some(&record.connection.owner_id)
            {
                return Err(McpError::Forbidden.into());
            }
            identity
        }
    };
    mcp::check_active(&record.connection, Utc::now())?;
    Ok(identity)
}

async fn recorded_answer<R: SessionRepository + McpRepository, B: TraceReader>(
    app: &App<R, B>,
    identity: &Identity,
    connection: Option<&McpConnection>,
    message: Value,
) -> Result<Option<Value>, McpHttpError> {
    let mut record = metadata(connection, Some(identity), &message);
    let started = std::time::Instant::now();
    let result = answer(app, scope(identity)?, message).await;
    let Some(response) = &result else {
        return Ok(None);
    };
    record.latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    record.outcome = if response.get("error").is_some() {
        McpOutcome::InvalidRequest
    } else if response.pointer("/result/isError") == Some(&Value::Bool(true)) {
        McpOutcome::ToolError
    } else {
        McpOutcome::Success
    };
    app.authentication.sessions.mcp_record(&record).await?;
    Ok(result)
}

async fn handle_request<R: SessionRepository + McpRepository, B: TraceReader>(
    app: Arc<App<R, B>>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
    named: Option<String>,
) -> Result<Response, McpHttpError> {
    let parsed = serde_json::from_slice::<Value>(&body);
    let (identity, connection) = authenticate(
        &app,
        &headers,
        &method,
        named.as_deref(),
        parsed.as_ref().unwrap_or(&Value::Null),
    )
    .await?;
    if let Err(error) = scope(&identity) {
        let mut record = metadata(
            connection.as_ref(),
            Some(&identity),
            parsed.as_ref().unwrap_or(&Value::Null),
        );
        record.outcome = McpOutcome::Forbidden;
        app.authentication.sessions.mcp_record(&record).await?;
        return Err(error.into());
    }
    Ok(match parsed {
        Ok(Value::Array(messages)) if (1..=BATCH_MAX).contains(&messages.len()) => {
            let mut answers = Vec::new();
            for message in messages {
                if let Some(answer) =
                    recorded_answer(&app, &identity, connection.as_ref(), message).await?
                {
                    answers.push(answer);
                }
            }
            respond((!answers.is_empty()).then(|| Value::Array(answers)))
        }
        Ok(message @ Value::Object(_)) => {
            respond(recorded_answer(&app, &identity, connection.as_ref(), message).await?)
        }
        _ => {
            let mut record = metadata(connection.as_ref(), Some(&identity), &Value::Null);
            record.outcome = McpOutcome::InvalidRequest;
            app.authentication.sessions.mcp_record(&record).await?;
            respond(Some(failure(
                Value::Null,
                -32600,
                "Invalid JSON-RPC request",
            )))
        }
    })
}
