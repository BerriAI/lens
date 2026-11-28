use std::sync::Arc;

use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, Method, header::CACHE_CONTROL},
    response::{IntoResponse, Response},
    routing::{delete, get},
};
use chrono::Utc;
use lens_auth::{
    Authentication, McpError, SessionRepository,
    mcp::{self, ActivityQuery, McpRepository},
};
use lens_contract::mcp::{McpConnectionCreated, McpCreateRequest, McpOverview};
use serde::Deserialize;

use crate::{auth, error::McpHttpError, routing::PublicRoutes};

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActivityParameters {
    hours: Option<u16>,
    connection: Option<String>,
    cursor: Option<String>,
    to_ms: Option<i64>,
}

pub fn router<R: SessionRepository + McpRepository + 'static>(
    authentication: Arc<Authentication<R>>,
) -> Router {
    Router::new()
        .public_route("/lens/connectors/mcp", get(overview::<R>).post(create::<R>))
        .public_route("/lens/connectors/mcp/{id}", delete(revoke::<R>))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .with_state(authentication)
}

async fn overview<R: SessionRepository + McpRepository>(
    State(auth): State<Arc<Authentication<R>>>,
    headers: HeaderMap,
    method: Method,
    Query(parameters): Query<ActivityParameters>,
) -> Result<Json<McpOverview>, McpHttpError> {
    let identity = auth::identity(&auth, &headers, &method).await?;
    if lens_auth::trace_read_scope(&identity).is_none() {
        return Err(McpError::Forbidden.into());
    }
    let now = Utc::now().timestamp_millis();
    let hours = parameters.hours.unwrap_or(168);
    if !(1..=720).contains(&hours) {
        return Err(McpError::Invalid("Choose a window from 1 to 720 hours").into());
    }
    let to_ms = parameters.to_ms.unwrap_or(now).min(now);
    let from_ms = to_ms
        .saturating_sub(i64::from(hours) * 3_600_000)
        .max(now - i64::from(mcp::RETENTION_DAYS) * 86_400_000);
    let before = parameters
        .cursor
        .map(|cursor| {
            let (at, id) = cursor
                .split_once(':')
                .ok_or(McpError::Invalid("Invalid activity cursor"))?;
            let at = at
                .parse::<i64>()
                .map_err(|_| McpError::Invalid("Invalid activity cursor"))?;
            let id = uuid::Uuid::parse_str(id)
                .map_err(|_| McpError::Invalid("Invalid activity cursor"))?;
            Ok::<_, McpError>((at, id.to_string()))
        })
        .transpose()?;
    if parameters
        .connection
        .as_ref()
        .is_some_and(|id| id.len() > 80)
    {
        return Err(McpError::Invalid("Invalid connection ID").into());
    }
    let all = mcp::can_manage_all(&identity);
    let owner = identity
        .user_id
        .clone()
        .filter(|id| !id.is_empty())
        .ok_or(McpError::Forbidden)?;
    let connections = auth
        .sessions
        .mcp_connections()
        .await?
        .into_iter()
        .map(|record| record.connection)
        .filter(|connection| all || connection.owner_id == owner)
        .collect();
    let activity = auth
        .sessions
        .mcp_activity(ActivityQuery {
            owner_id: (!all).then_some(owner),
            connection_id: parameters.connection,
            from_ms,
            to_ms,
            before,
        })
        .await?;
    Ok(Json(McpOverview {
        connections,
        activity,
        authentication: mcp::authentication(&identity),
        can_create: mcp::can_create(&identity),
        can_manage_all: all,
        scope: mcp::scope_label(&identity).into(),
        gateway_url: auth
            .settings
            .gateway_login_url()
            .and_then(|url| url.strip_suffix("/lens/login").map(str::to_owned)),
        from_ms,
        to_ms,
        retention_days: mcp::RETENTION_DAYS,
    }))
}

async fn create<R: SessionRepository + McpRepository>(
    State(auth): State<Arc<Authentication<R>>>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Response, McpHttpError> {
    let identity = auth::identity(&auth, &headers, &method).await?;
    let request: McpCreateRequest = serde_json::from_slice(&body)
        .map_err(|_| McpError::Invalid("Invalid MCP connection request"))?;
    let created: McpConnectionCreated =
        mcp::create(&auth.sessions, &identity, request, Utc::now()).await?;
    Ok(([(CACHE_CONTROL, "no-store")], Json(created)).into_response())
}

async fn revoke<R: SessionRepository + McpRepository>(
    State(auth): State<Arc<Authentication<R>>>,
    headers: HeaderMap,
    method: Method,
    Path(id): Path<String>,
) -> Result<Json<bool>, McpHttpError> {
    let identity = auth::identity(&auth, &headers, &method).await?;
    let records = auth.sessions.mcp_connections().await?;
    let record = records
        .iter()
        .find(|record| record.connection.id == id)
        .ok_or(McpError::NotFound)?;
    mcp::authorize_manager(&identity, &record.connection)?;
    auth.sessions.mcp_revoke(&id, Utc::now()).await?;
    Ok(Json(true))
}
