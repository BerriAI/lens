use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, Method},
    routing::{get, post},
};
use lens_auth::{Authentication, SessionRepository};
use lens_contract::{
    auth::{Identity, Role},
    optimizations::*,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use super::{
    Optimizations,
    store::{Attempt, AttemptState, attempt_prefix},
};
use crate::error::OptimizationError as Error;

struct App<R> {
    authentication: Arc<Authentication<R>>,
    engine: Optimizations,
}

pub fn router<R: SessionRepository + 'static>(
    authentication: Arc<Authentication<R>>,
    engine: Optimizations,
) -> Router {
    Router::new()
        .route("/lens/optimizations", get(list::<R>).post(create::<R>))
        .route("/lens/optimizations/models", get(models::<R>))
        .route("/lens/optimizations/assign", post(assign::<R>))
        .route("/lens/optimizations/{id}", get(read::<R>))
        .route("/lens/optimizations/{id}/status", post(status::<R>))
        .route("/lens/optimizations/{id}/claim", post(claim::<R>))
        .route("/lens/optimizations/{id}/submit", post(submit::<R>))
        .layer(DefaultBodyLimit::max(2_000_000))
        .with_state(Arc::new(App {
            authentication,
            engine,
        }))
}

async fn admin<R: SessionRepository>(
    app: &App<R>,
    headers: &HeaderMap,
    method: Method,
) -> Result<Identity, Error> {
    let identity = lens_server::auth::identity(&app.authentication, headers, &method).await?;
    if identity.user_role != Role::ProxyAdmin
        && !(method == Method::GET && identity.user_role == Role::ProxyAdminViewer)
    {
        return Err(Error::Forbidden);
    }
    Ok(identity)
}

async fn list<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
) -> Result<Json<Vec<OptimizationRun>>, Error> {
    admin(&app, &headers, Method::GET).await?;
    let mut runs = Vec::new();
    for record in app.engine.0.store.records().await? {
        runs.push(app.engine.0.store.view(record).await?);
    }
    runs.sort_by_key(|run| std::cmp::Reverse(run.created_at));
    Ok(Json(runs))
}

async fn models<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
) -> Result<Json<Value>, Error> {
    admin(&app, &headers, Method::GET).await?;
    Ok(Json(json!({"data": app.engine.models().await?})))
}

async fn create<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
    Json(config): Json<OptimizationConfig>,
) -> Result<Json<OptimizationRun>, Error> {
    admin(&app, &headers, Method::POST).await?;
    app.engine.create(config).await.map(Json)
}

async fn read<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<OptimizationRun>, Error> {
    admin(&app, &headers, Method::GET).await?;
    let (_, record) = app.engine.0.store.get(&id).await?;
    app.engine.0.store.view(record).await.map(Json)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusRequest {
    status: OptimizationStatus,
}

async fn status<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<StatusRequest>,
) -> Result<Json<OptimizationRun>, Error> {
    admin(&app, &headers, Method::POST).await?;
    app.engine.status(&id, request.status).await.map(Json)
}

async fn assign<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
    Json(request): Json<OptimizationAssignmentRequest>,
) -> Result<Json<OptimizationAssignment>, Error> {
    let identity =
        lens_server::auth::identity(&app.authentication, &headers, &Method::POST).await?;
    app.engine.assign(&identity, request).await.map(Json)
}

async fn claim<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, Error> {
    admin(&app, &headers, Method::POST).await?;
    let (_, record) = app.engine.0.store.get(&id).await?;
    if record.config.scope != OptimizationScope::Agent {
        return Err(Error::Invalid(
            "This optimization does not use an agent runner",
        ));
    }
    if record.status != OptimizationStatus::Running {
        return Ok(Json(Value::Null));
    }
    for (snapshot, attempt) in app.engine.0.store.attempts(&id).await? {
        if attempt.external
            && let Some(attempt) = app.engine.0.store.claim(snapshot, attempt).await?
        {
            return Ok(Json(
                json!({"id":attempt.result.id,"lease":attempt.owner,"model":attempt.result.model,"input":attempt.result.input,"expected":attempt.result.expected,"requestId":format!("{}-{}", id, attempt.result.id)}),
            ));
        }
    }
    Ok(Json(Value::Null))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Submission {
    id: String,
    lease: String,
    trace_id: Option<String>,
    error: Option<String>,
}

async fn submit<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(request): Json<Submission>,
) -> Result<Json<Value>, Error> {
    admin(&app, &headers, Method::POST).await?;
    let (_, record) = app.engine.0.store.get(&id).await?;
    if request.id.len() != 64 || !request.id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::NotFound);
    }
    let snapshot = app
        .engine
        .0
        .store
        .0
        .read(&format!("{}{}", attempt_prefix(&id)?, request.id))
        .await?;
    if snapshot.value.is_null() {
        return Err(Error::NotFound);
    }
    let mut attempt: Attempt = serde_json::from_value(snapshot.value.clone())?;
    if !attempt.external
        || attempt.state != AttemptState::Running
        || attempt.owner.as_deref() != Some(&request.lease)
        || attempt.deadline < chrono::Utc::now().timestamp_millis()
    {
        return Err(Error::Conflict);
    }
    if request.error.is_some() {
        attempt.result.error = Some("The connected agent failed; inspect its local run log".into());
        app.engine.0.store.finish(&id, attempt).await?;
        return Ok(Json(json!({"accepted":true})));
    }
    let trace_id = request
        .trace_id
        .filter(|id| !id.is_empty() && id.len() <= 128)
        .ok_or(Error::Invalid("Return the completed agent trace ID"))?;
    if let Some(source) = app.engine.trace_source(&record, &trace_id, "").await? {
        if source.request_id.as_deref() != Some(&format!("{}-{}", id, attempt.result.id))
            || source.started_at < attempt.deadline - 600_000
        {
            return Err(Error::Invalid(
                "The agent trace does not match this trial; record lens.optimization.request_id on its root span",
            ));
        }
        if source.served_model.as_deref() != Some(&attempt.result.model) {
            return Err(Error::Invalid(
                "The agent trace does not confirm the requested candidate model",
            ));
        }
        attempt.result.output = Some(source.original);
        attempt.result.trace_id = trace_id;
        attempt.result.span_id = source.span_id;
        attempt.result.served_model = source.served_model;
        attempt.result.cost_usd = source.cost;
        attempt.result.duration_ms = source.duration;
        attempt.context = source.context;
        attempt.external = false;
        attempt.state = AttemptState::Pending;
        attempt.owner = None;
        app.engine.0.store.save(snapshot, &attempt).await?;
        return Ok(Json(json!({"accepted":true})));
    }
    Ok(Json(json!({"accepted":false,"waitingForTrace":true})))
}
