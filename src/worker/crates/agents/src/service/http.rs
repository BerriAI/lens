use super::{Control, tools};
use crate::{
    Error,
    conversations::{Access, BotInput},
    model::{AnnotationFilter, TraceIdentity},
};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use lens_auth::{Authentication, SessionRepository};
use lens_server::tracing::{TraceConfig, TraceReader};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

struct App<R, B> {
    auth: Arc<Authentication<R>>,
    reader: B,
    config: TraceConfig,
    control: Control,
}

pub fn router<R: SessionRepository + 'static, B: TraceReader + 'static>(
    auth: Arc<Authentication<R>>,
    reader: B,
    config: TraceConfig,
    control: Control,
) -> Router {
    Router::new()
        .route("/lens/workspace", get(workspace::<R, B>))
        .route("/lens/workspace/models", get(workspace_models::<R, B>))
        .route(
            "/lens/workspace/slack",
            get(slack_status::<R, B>).post(connect_slack::<R, B>),
        )
        .route("/lens/workspace/activity", get(activity::<R, B>))
        .route("/lens/workspace/bots", post(create_bot::<R, B>))
        .route("/lens/workspace/bots/{id}", patch(bot_state::<R, B>))
        .route(
            "/lens/workspace/conversations",
            get(conversations::<R, B>).post(create_conversation::<R, B>),
        )
        .route(
            "/lens/workspace/conversations/{id}",
            get(conversation::<R, B>),
        )
        .route(
            "/lens/workspace/conversations/{id}/messages",
            post(message::<R, B>),
        )
        .route(
            "/lens/workspace/conversations/{id}/stop",
            post(stop::<R, B>),
        )
        .route(
            "/lens/workspace/conversations/{id}/slack",
            get(slack_link::<R, B>)
                .post(slack_code::<R, B>)
                .delete(unlink_slack::<R, B>),
        )
        .route(
            "/lens/workspace/annotations/query",
            post(annotations::<R, B>),
        )
        .route("/lens/workspace/reports", get(reports::<R, B>))
        .route("/lens/workspace/reports/{id}", get(report::<R, B>))
        .route("/lens/workspace/artifacts/{id}", get(artifact::<R, B>))
        .route(
            "/agent-runtime/mcp",
            post(mcp::<R, B>).layer(axum::extract::DefaultBodyLimit::max(16 * 1024 * 1024)),
        )
        .route("/agent-runtime/v1/models", get(models::<R, B>))
        .route(
            "/agent-runtime/v1/chat/completions",
            post(completions::<R, B>),
        )
        .route("/agent-runtime/v1/responses", post(responses::<R, B>))
        .layer(axum::extract::DefaultBodyLimit::max(4 * 1024 * 1024))
        .with_state(Arc::new(App {
            auth,
            reader,
            config,
            control,
        }))
}

async fn access<R: SessionRepository, B>(
    app: &App<R, B>,
    headers: &HeaderMap,
    method: &Method,
) -> Result<Access, Error> {
    let identity = lens_server::auth::identity(&app.auth, headers, method).await?;
    Access::for_identity(&identity)
}

async fn workspace<R: SessionRepository, B>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
) -> Result<Json<Value>, Error> {
    let access = access(&app, &headers, &Method::GET).await?;
    Ok(Json(
        json!({"agents":app.control.store().agents(&access).await?,"model":app.control.0.model,"writable":access.writable,"slack_configured":app.control.slack_configured()}),
    ))
}

async fn slack_status<R: SessionRepository, B>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
) -> Result<Json<Value>, Error> {
    access(&app, &headers, &Method::GET)
        .await?
        .require_write()?;
    Ok(Json(app.control.slack_status()))
}

async fn connect_slack<R: SessionRepository, B>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    Json(input): Json<crate::slack::setup::Credentials>,
) -> Result<Json<Value>, Error> {
    let access = access(&app, &headers, &Method::POST).await?;
    app.control.connect_slack(&access, input).await?;
    Ok(Json(json!({"configured":true,"connected":false})))
}

async fn create_bot<R: SessionRepository, B>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    Json(input): Json<BotInput>,
) -> Result<Json<Value>, Error> {
    let access = access(&app, &headers, &Method::POST).await?;
    Ok(Json(json!(app.control.create_bot(&access, &input).await?)))
}

async fn workspace_models<R: SessionRepository, B>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
) -> Result<Json<Vec<String>>, Error> {
    access(&app, &headers, &Method::GET)
        .await?
        .require_write()?;
    Ok(Json(app.control.available_models().await?))
}

async fn activity<R: SessionRepository, B>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
) -> Result<Json<Value>, Error> {
    let access = access(&app, &headers, &Method::GET).await?;
    Ok(Json(json!(
        app.control
            .store()
            .annotation_activity(&access.workspace)
            .await?
    )))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Enabled {
    enabled: bool,
}
async fn bot_state<R: SessionRepository, B>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Enabled>,
) -> Result<Json<Value>, Error> {
    let access = access(&app, &headers, &Method::PATCH).await?;
    Ok(Json(json!(
        app.control.enabled(&access, &id, input.enabled).await?
    )))
}

#[derive(Deserialize)]
struct AgentQuery {
    actor_id: String,
}
async fn conversations<R: SessionRepository, B>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    Query(input): Query<AgentQuery>,
) -> Result<Json<Value>, Error> {
    let access = access(&app, &headers, &Method::GET).await?;
    Ok(Json(json!(
        app.control
            .store()
            .conversations(&access, &input.actor_id)
            .await?
    )))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NewConversation {
    id: Option<String>,
    model: Option<String>,
    actor_id: Option<String>,
    title: String,
}
async fn create_conversation<R: SessionRepository, B>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    Json(input): Json<NewConversation>,
) -> Result<Json<Value>, Error> {
    let access = access(&app, &headers, &Method::POST).await?;
    access.require_write()?;
    let actor_id = match input.actor_id {
        Some(id) => id,
        None => app.control.main_agent(&access).await?.id,
    };
    let actor = app
        .control
        .store()
        .agent(&access.workspace, &actor_id)
        .await?;
    let model = if matches!(actor.runtime, crate::conversations::Runtime::Hermes) {
        actor.model
    } else {
        Some(app.control.selected_model(input.model.as_deref()).await?)
    };
    Ok(Json(json!(
        app.control
            .store()
            .create_conversation_with_model(
                &access,
                &actor_id,
                &input.title,
                &input.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                model.as_deref(),
            )
            .await?
    )))
}

#[derive(Default, Deserialize)]
struct EventQuery {
    #[serde(default)]
    after: i64,
}
async fn conversation<R: SessionRepository, B>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(input): Query<EventQuery>,
) -> Result<Json<Value>, Error> {
    let access = access(&app, &headers, &Method::GET).await?;
    let conversation = app.control.store().conversation(&access, &id).await?;
    let agent = app
        .control
        .store()
        .agent(&access.workspace, &conversation.actor_id)
        .await?;
    let history = if matches!(agent.runtime, crate::conversations::Runtime::Hermes) {
        app.control.history(&conversation).await?
    } else {
        Value::Null
    };
    Ok(Json(json!({
        "conversation":conversation,
        "native_history":history,
        "turns":app.control.store().turns(&access,&id).await?,
        "events":app.control.store().events(&access,&id,input.after).await?
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Message {
    id: String,
    text: String,
}
async fn message<R: SessionRepository, B>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<Message>,
) -> Result<Json<Value>, Error> {
    let access = access(&app, &headers, &Method::POST).await?;
    let (turn, created) = app
        .control
        .store()
        .admit(&access, &id, &input.id, &input.text)
        .await?;
    if created {
        app.control.run(&access, turn.clone()).await?;
    }
    Ok(Json(json!(turn)))
}

async fn stop<R: SessionRepository, B>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, Error> {
    let access = access(&app, &headers, &Method::POST).await?;
    app.control.stop(&access, &id).await?;
    Ok(Json(json!({"stopped":true})))
}

async fn slack_link<R: SessionRepository, B>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, Error> {
    let access = access(&app, &headers, &Method::GET).await?;
    Ok(Json(app.control.store().slack_link(&access, &id).await?))
}
async fn slack_code<R: SessionRepository, B>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, Error> {
    let access = access(&app, &headers, &Method::POST).await?;
    if !app.control.slack_configured() {
        return Err(Error::Invalid(
            "Your administrator needs to configure the Lens Slack app first",
        ));
    }
    Ok(Json(json!(
        app.control.store().slack_code(&access, &id).await?
    )))
}
async fn unlink_slack<R: SessionRepository, B>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, Error> {
    let access = access(&app, &headers, &Method::DELETE).await?;
    app.control.store().unlink_slack(&access, &id).await?;
    Ok(Json(json!({"linked":false})))
}

async fn annotations<R: SessionRepository, B>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    Json(input): Json<AnnotationFilter>,
) -> Result<Json<Value>, Error> {
    let access = access(&app, &headers, &Method::POST).await?;
    Ok(Json(json!(
        app.control
            .store()
            .annotations(&access.workspace, &input)
            .await?
    )))
}

#[derive(Default, Deserialize)]
struct ReportQuery {
    trace_id: Option<String>,
    trace_ref: Option<String>,
    before: Option<String>,
}
async fn reports<R: SessionRepository, B>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    Query(input): Query<ReportQuery>,
) -> Result<Json<Value>, Error> {
    let access = access(&app, &headers, &Method::GET).await?;
    let trace = match (input.trace_id, input.trace_ref) {
        (Some(trace_id), Some(trace_ref)) => Some(TraceIdentity {
            trace_id,
            trace_ref,
        }),
        (None, None) => None,
        _ => return Err(Error::Invalid("Pass both trace_id and trace_ref")),
    };
    Ok(Json(json!(
        app.control
            .store()
            .reports(&access.workspace, trace.as_ref(), input.before.as_deref())
            .await?
    )))
}
async fn report<R: SessionRepository, B>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, Error> {
    let access = access(&app, &headers, &Method::GET).await?;
    Ok(Json(json!(
        app.control.store().report(&access.workspace, &id).await?
    )))
}

async fn artifact<R: SessionRepository, B>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, Error> {
    let access = access(&app, &headers, &Method::GET).await?;
    Ok(Json(json!(
        app.control.store().artifact(&access.workspace, &id).await?
    )))
}

async fn mcp<R: SessionRepository, B: TraceReader>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    Json(request): Json<Value>,
) -> Result<Response, Error> {
    let agent = app.control.authenticate(&headers).await?;
    let Some(id) = request.get("id") else {
        return Ok(StatusCode::ACCEPTED.into_response());
    };
    let result = match request["method"].as_str() {
        Some("initialize") => {
            json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"lens","version":env!("CARGO_PKG_VERSION")},"instructions":super::MAIN_INSTRUCTIONS})
        }
        Some("ping") => json!({}),
        Some("tools/list") => json!({"tools":tools::catalog()}),
        Some("tools/call") => {
            let name = request
                .pointer("/params/name")
                .and_then(Value::as_str)
                .ok_or(Error::Invalid("Tool name required"))?;
            let arguments = request
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let (value, failed) = match tools::call(
                &app.control,
                &app.reader,
                app.config,
                &agent,
                name,
                arguments,
            )
            .await
            {
                Ok(value) => (value, false),
                Err(error) => (json!({"error":error.to_string()}), true),
            };
            json!({"content":[{"type":"text","text":serde_json::to_string(&value)?}],"isError":failed})
        }
        _ => return Ok(Json(
            json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Method not found"}}),
        )
        .into_response()),
    };
    Ok(Json(json!({"jsonrpc":"2.0","id":id,"result":result})).into_response())
}

async fn models<R: SessionRepository, B>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
) -> Result<Json<Value>, Error> {
    app.control.authenticate(&headers).await?;
    let models = app.control.available_models().await?;
    Ok(Json(
        json!({"object":"list","data":models.into_iter().map(|id|json!({"id":id,"object":"model"})).collect::<Vec<_>>()}),
    ))
}
async fn completions<R: SessionRepository, B>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, Error> {
    inference(&app, &headers, body, "chat/completions").await
}
async fn responses<R: SessionRepository, B>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response, Error> {
    inference(&app, &headers, body, "responses").await
}
async fn inference<R, B>(
    app: &App<R, B>,
    headers: &HeaderMap,
    mut body: Value,
    path: &str,
) -> Result<Response, Error> {
    let agent = app.control.authenticate(headers).await?;
    crate::model::bounded_text(
        body.get("model")
            .and_then(Value::as_str)
            .ok_or(Error::Invalid("Choose a model"))?,
        256,
    )?;
    validate_inference(&body)?;
    let object = body
        .as_object_mut()
        .ok_or(Error::Invalid("Model request must be an object"))?;
    object.insert("user".into(), json!(agent.owner));
    object.insert(
        "metadata".into(),
        json!({"lens_agent_id":agent.id,"lens_workspace":agent.workspace}),
    );
    let response = app
        .control
        .0
        .client
        .post(format!("{}/{path}", app.control.0.gateway_base))
        .bearer_auth(&app.control.0.gateway_key)
        .json(&body)
        .send()
        .await
        .map_err(|_| Error::Runtime)?;
    if !response.status().is_success() {
        return Ok((StatusCode::BAD_GATEWAY,Json(json!({"error":{"message":"The LiteLLM gateway rejected this model request","upstream_status":response.status().as_u16()}}))).into_response());
    }
    let content_type = response.headers().get("content-type").cloned();
    let mut result = Response::new(Body::from_stream(response.bytes_stream()));
    if let Some(content_type) = content_type {
        result.headers_mut().insert("content-type", content_type);
    }
    Ok(result)
}

fn validate_inference(body: &Value) -> Result<(), Error> {
    let object = body
        .as_object()
        .ok_or(Error::Invalid("Model request must be an object"))?;
    let allowed = [
        "model",
        "messages",
        "input",
        "instructions",
        "tools",
        "tool_choice",
        "parallel_tool_calls",
        "stream",
        "stream_options",
        "temperature",
        "top_p",
        "n",
        "stop",
        "max_tokens",
        "max_completion_tokens",
        "max_output_tokens",
        "presence_penalty",
        "frequency_penalty",
        "logit_bias",
        "logprobs",
        "top_logprobs",
        "response_format",
        "seed",
        "user",
        "metadata",
        "reasoning",
        "reasoning_effort",
        "verbosity",
        "text",
        "store",
        "previous_response_id",
        "include",
        "truncation",
        "service_tier",
        "prompt_cache_key",
        "prompt_cache_retention",
        "safety_identifier",
        "modalities",
        "audio",
        "prediction",
        "thinking",
        "output_config",
    ];
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(Error::Invalid(
            "Only standard model parameters are allowed; gateway routing and credentials are managed by Lens",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::api_key("api_key")]
    #[case::api_base("api_base")]
    #[case::custom_provider("custom_llm_provider")]
    #[case::nested_override("extra_body")]
    #[case::fallback("fallbacks")]
    #[case::provider_headers("extra_headers")]
    fn runtimes_cannot_override_gateway_routing(#[case] key: &str) {
        assert!(validate_inference(&json!({"model":"test-model",key:{"value":"test"}})).is_err());
    }

    #[rstest]
    fn standard_native_requests_keep_tools_and_reasoning() {
        assert!(validate_inference(&json!({"model":"test-model","messages":[],"stream":true,"tools":[],"reasoning_effort":"high"})).is_ok());
    }
}
