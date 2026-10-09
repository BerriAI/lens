use std::{future::Future, sync::Arc};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, RawQuery, Request, State},
    http::{HeaderMap, Method},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use lens_auth::{Authentication, ReadScope, SessionRepository, trace_read_scope};
use litellm_traces::{
    QueryScope, SpanDetail, SpanErrorPage, Trace, TracePage,
    query::named::ReadAccessParams,
    request::{TraceDetailRequest, TraceErrorPageRequest, TraceSpanRequest},
};
use serde::Serialize;

pub use crate::error::TraceReadError;
use crate::{auth, error::TraceHttpError};
mod validation;

#[derive(Clone, Debug, Serialize)]
pub struct TraceAgent {
    pub name: String,
    pub runs: u64,
    pub failed_runs: u64,
    pub last_seen: DateTime<Utc>,
    pub frameworks: Vec<String>,
}

#[derive(Serialize)]
pub struct TraceAgentList {
    pub agents: Vec<TraceAgent>,
}

pub trait TraceReader: Send + Sync {
    type Help: Serialize + Send;

    fn list(
        &self,
        scope: ReadAccessParams,
        start_ms: i64,
        end_ms: i64,
        cursor: Option<String>,
        limit: u32,
    ) -> impl Future<Output = Result<TracePage, TraceReadError>> + Send;
    fn agents(
        &self,
        scope: ReadAccessParams,
        start_ms: i64,
        end_ms: i64,
        limit: u32,
    ) -> impl Future<Output = Result<Vec<TraceAgent>, TraceReadError>> + Send;
    fn trace(
        &self,
        scope: ReadAccessParams,
        trace_id: String,
        request: TraceDetailRequest,
    ) -> impl Future<Output = Result<Option<Trace>, TraceReadError>> + Send;
    fn span(
        &self,
        scope: ReadAccessParams,
        trace_id: String,
        span_id: String,
        request: TraceSpanRequest,
    ) -> impl Future<Output = Result<Option<SpanDetail>, TraceReadError>> + Send;
    fn span_error(
        &self,
        scope: ReadAccessParams,
        trace_id: String,
        span_id: String,
        request: TraceErrorPageRequest,
    ) -> impl Future<Output = Result<Option<SpanErrorPage>, TraceReadError>> + Send;
    fn query(
        &self,
        scope: QueryScope,
        sql: String,
    ) -> impl Future<Output = Result<serde_json::Value, TraceReadError>> + Send;
    fn help(
        &self,
        scope: QueryScope,
    ) -> impl Future<Output = Result<Self::Help, TraceReadError>> + Send;
}

#[derive(Clone, Copy, Debug)]
pub struct TraceConfig {
    pub list_limit: u32,
    pub agent_limit: u32,
    pub retention_days: i64,
    pub retry_after_seconds: i64,
}

impl Default for TraceConfig {
    fn default() -> Self {
        Self {
            list_limit: 50,
            agent_limit: 500,
            retention_days: 14,
            retry_after_seconds: 2,
        }
    }
}

struct App<R, B> {
    authentication: Arc<Authentication<R>>,
    reader: B,
    config: TraceConfig,
}

pub fn router<R: SessionRepository + 'static, B: TraceReader + 'static>(
    authentication: Arc<Authentication<R>>,
    reader: B,
    config: TraceConfig,
) -> Router {
    Router::new()
        .route("/v1/traces", get(list::<R, B>))
        .route("/v1/traces/agents", get(agents::<R, B>))
        .route("/v1/traces/query", post(query::<R, B>))
        .route("/v1/traces/query/help", get(help::<R, B>))
        .route("/v1/traces/{trace_id}", get(trace::<R, B>))
        .route("/v1/traces/{trace_id}/spans/{span_id}", get(span::<R, B>))
        .route(
            "/v1/traces/{trace_id}/spans/{span_id}/error",
            get(span_error::<R, B>),
        )
        .layer(DefaultBodyLimit::disable())
        .layer(middleware::from_fn(redirect_trailing_slash))
        .with_state(Arc::new(App {
            authentication,
            reader,
            config,
        }))
}

pub(crate) async fn redirect_trailing_slash(request: Request, next: Next) -> Response {
    let path = request.uri().path();
    let normalized = path.trim_end_matches('/');
    let parts: Vec<_> = normalized.split('/').collect();
    let known = matches!(
        parts.as_slice(),
        ["", "v1", "traces"]
            | ["", "v1", "traces", _]
            | ["", "v1", "traces", "query", "help"]
            | ["", "v1", "traces", _, "spans", _]
            | ["", "v1", "traces", _, "spans", _, "error"]
            | ["", "lens", "tracing", "keys"]
            | ["", "lens", "tracing", "keys", _]
            | ["", "lens", "service"]
    );
    if path != normalized && known {
        let query = request
            .uri()
            .query()
            .map(|query| format!("?{query}"))
            .unwrap_or_default();
        let host = request
            .headers()
            .get(axum::http::header::HOST)
            .and_then(|host| host.to_str().ok());
        let location = if let Some(host) = host {
            format!(
                "{}://{host}{normalized}{query}",
                request.uri().scheme_str().unwrap_or("http")
            )
        } else {
            format!("{normalized}{query}")
        };
        return (
            axum::http::StatusCode::TEMPORARY_REDIRECT,
            [(axum::http::header::LOCATION, location)],
        )
            .into_response();
    }
    next.run(request).await
}

fn scope(identity: &lens_contract::auth::Identity) -> Result<ReadAccessParams, TraceHttpError> {
    match trace_read_scope(identity) {
        Some(ReadScope::AllRows) => Ok(ReadAccessParams {
            all_teams: true,
            user_id: String::new(),
            team_ids: Vec::new(),
        }),
        Some(ReadScope::OwnedRows { user_id, team_ids }) => Ok(ReadAccessParams {
            all_teams: false,
            user_id,
            team_ids,
        }),
        None => Err(TraceHttpError::Forbidden(
            "Not allowed to view agent traces",
        )),
    }
}

fn query_scope(identity: &lens_contract::auth::Identity) -> Result<QueryScope, TraceHttpError> {
    match trace_read_scope(identity) {
        Some(ReadScope::AllRows) => Ok(QueryScope::All),
        Some(ReadScope::OwnedRows { user_id, team_ids }) => {
            Ok(QueryScope::Owned { user_id, team_ids })
        }
        None => Err(TraceHttpError::Forbidden("Not allowed to view logs")),
    }
}

fn read_error(source: TraceReadError, config: TraceConfig) -> TraceHttpError {
    TraceHttpError::Read {
        source,
        retry_after_seconds: config.retry_after_seconds,
    }
}

async fn list<R: SessionRepository, B: TraceReader>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    method: Method,
    RawQuery(query): RawQuery,
) -> Result<Json<TracePage>, TraceHttpError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let request = validation::list(query.as_deref(), app.config)?;
    let now = Utc::now().timestamp_millis();
    let page = app
        .reader
        .list(
            scope(&identity)?,
            request.start_ms.unwrap_or(now - 86_400_000),
            request.end_ms.unwrap_or(now),
            request.cursor,
            app.config.list_limit,
        )
        .await
        .map_err(|error| read_error(error, app.config))?;
    Ok(Json(page))
}

async fn agents<R: SessionRepository, B: TraceReader>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    method: Method,
    RawQuery(query): RawQuery,
) -> Result<Json<TraceAgentList>, TraceHttpError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let (start_ms, end_ms) = validation::agents(query.as_deref(), app.config)?;
    let now = Utc::now().timestamp_millis();
    let start = now.saturating_sub(app.config.retention_days.saturating_mul(86_400_000));
    let agents = app
        .reader
        .agents(
            scope(&identity)?,
            start_ms.unwrap_or(start),
            end_ms.unwrap_or(now),
            app.config.agent_limit,
        )
        .await
        .map_err(|error| read_error(error, app.config))?;
    Ok(Json(TraceAgentList { agents }))
}

async fn trace<R: SessionRepository, B: TraceReader>(
    State(app): State<Arc<App<R, B>>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    method: Method,
    RawQuery(query): RawQuery,
) -> Result<Json<Trace>, TraceHttpError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let request = validation::detail(query.as_deref())?;
    let trace = app
        .reader
        .trace(scope(&identity)?, id.clone(), request)
        .await
        .map_err(|error| read_error(error, app.config))?
        .ok_or(TraceHttpError::MissingTrace(id))?;
    Ok(Json(trace))
}

async fn span<R: SessionRepository, B: TraceReader>(
    State(app): State<Arc<App<R, B>>>,
    Path((id, span)): Path<(String, String)>,
    headers: HeaderMap,
    method: Method,
    RawQuery(query): RawQuery,
) -> Result<Json<SpanDetail>, TraceHttpError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let request = validation::span(query.as_deref());
    let value = app
        .reader
        .span(scope(&identity)?, id, span.clone(), request)
        .await
        .map_err(|error| read_error(error, app.config))?
        .ok_or(TraceHttpError::MissingSpan(span))?;
    Ok(Json(value))
}

async fn span_error<R: SessionRepository, B: TraceReader>(
    State(app): State<Arc<App<R, B>>>,
    Path((id, span)): Path<(String, String)>,
    headers: HeaderMap,
    method: Method,
    RawQuery(query): RawQuery,
) -> Result<Json<SpanErrorPage>, TraceHttpError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let request = validation::error_page(query.as_deref())?;
    let value = app
        .reader
        .span_error(scope(&identity)?, id, span, request)
        .await
        .map_err(|error| read_error(error, app.config))?
        .ok_or(TraceHttpError::MissingDiagnostic)?;
    Ok(Json(value))
}

async fn query<R: SessionRepository, B: TraceReader>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    method: Method,
    body: axum::body::Bytes,
) -> Result<Json<serde_json::Value>, TraceHttpError> {
    use crate::datasets::validation::{Model, parse_body, request};
    let parsed = parse_body(&body, &headers)?;
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let scope = query_scope(&identity)?;
    let request: litellm_traces::request::TraceQueryRequest = request(parsed, Model::Query)?;
    Ok(Json(
        app.reader
            .query(scope, request.sql)
            .await
            .map_err(TraceHttpError::Query)?,
    ))
}

async fn help<R: SessionRepository, B: TraceReader>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<B::Help>, TraceHttpError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    Ok(Json(
        app.reader
            .help(query_scope(&identity)?)
            .await
            .map_err(|_| TraceHttpError::Help)?,
    ))
}
