use crate::routing::PublicRoutes;
pub(crate) mod validation;

pub use crate::error::{ActivityError, ActivityReadError};
use crate::{auth, datasets::validation as body_validation, error::InvestigationError};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, RawQuery, State},
    http::{HeaderMap, Method},
    routing::{get, post},
};
use chrono::{DateTime, Datelike, TimeDelta, Utc};
use lens_auth::{Authentication, SessionRepository};
use lens_contract::{
    activity::{ActivityAvailability, ActivitySelection, Preview},
    auth::{Identity, Role},
    execution::ExecutionId,
    investigations::{Public, Scope},
    worker::{Execution, ExecutionContent, ExecutionSource, Sample},
};
use lens_investigations::{LensRepository, TraceFindingsRepository, can_access};
use serde_json::json;
use std::{future::Future, sync::Arc};

pub struct PreviewWindow {
    pub start: u64,
    pub end: u64,
    pub offset: u64,
}

pub trait ActivityReader: Send + Sync {
    fn availability(
        &self,
        scope: &Scope,
    ) -> impl Future<Output = Result<ActivityAvailability, ActivityReadError>> + Send;
    fn agents(
        &self,
        scope: &Scope,
    ) -> impl Future<Output = Result<Vec<String>, ActivityReadError>> + Send;
    fn preview(
        &self,
        scope: &Scope,
        selection: &ActivitySelection,
        window: PreviewWindow,
    ) -> impl Future<Output = Result<Sample, ActivityReadError>> + Send;
    fn content(
        &self,
        scope: &Scope,
        execution: &Execution,
        cursor: &str,
        offset: u32,
    ) -> impl Future<Output = Result<ExecutionContent, ActivityReadError>> + Send;
}

struct App<R, D, S> {
    authentication: Arc<Authentication<R>>,
    repository: D,
    sources: Option<S>,
}

pub fn router<R, D, S>(
    authentication: Arc<Authentication<R>>,
    repository: D,
    sources: Option<S>,
) -> Router
where
    R: SessionRepository + 'static,
    D: LensRepository + TraceFindingsRepository + 'static,
    S: ActivityReader + 'static,
{
    Router::new()
        .public_route("/lens/activity/available", get(available::<R, D, S>))
        .public_route("/lens/agents", get(agents::<R, D, S>))
        .public_route("/lens/preview/sample", post(preview::<R, D, S>))
        .public_route("/lens/traces/findings", post(findings::<R, D, S>))
        .public_route(
            "/lens/{lens_id}/executions/{execution_id}",
            get(content::<R, D, S>),
        )
        .layer(DefaultBodyLimit::disable())
        .layer(axum::middleware::from_fn(
            crate::routing::redirect_trailing_slash,
        ))
        .with_state(Arc::new(App {
            authentication,
            repository,
            sources,
        }))
}

async fn findings<R: SessionRepository, D: TraceFindingsRepository, S: ActivityReader>(
    State(app): State<Arc<App<R, D, S>>>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Json<Vec<lens_contract::investigations::TraceFindingCount>>, ActivityError> {
    let parsed = body_validation::parse_body(&body, &headers)?;
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let request: lens_contract::feedback::TraceFeedbackRequest = body_validation::request(
        parsed,
        body_validation::Model::Feedback(crate::feedback::validation::Model::Summary),
    )?;
    scope(&identity)?;
    Ok(Json(
        app.repository
            .trace_findings(&request.traces)
            .await
            .map_err(InvestigationError::from)?,
    ))
}

fn scope(identity: &Identity) -> Result<Scope, InvestigationError> {
    if !matches!(
        identity.user_role,
        Role::ProxyAdmin | Role::ProxyAdminViewer
    ) {
        return Err(InvestigationError::ForbiddenRead);
    }
    Ok(Scope {
        all_teams: true,
        ..Scope::default()
    })
}

async fn available<R: SessionRepository, D: LensRepository, S: ActivityReader>(
    State(app): State<Arc<App<R, D, S>>>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<ActivityAvailability>, ActivityError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let scope = scope(&identity)?;
    Ok(Json(match &app.sources {
        Some(sources) => sources.availability(&scope).await?,
        None => ActivityAvailability::default(),
    }))
}

async fn agents<R: SessionRepository, D: LensRepository, S: ActivityReader>(
    State(app): State<Arc<App<R, D, S>>>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<Vec<String>>, ActivityError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let scope = scope(&identity)?;
    Ok(Json(match &app.sources {
        Some(sources) => sources.agents(&scope).await?,
        None => Vec::new(),
    }))
}

fn window(request: &Preview, now: DateTime<Utc>) -> Result<PreviewWindow, ActivityError> {
    let now = request.as_of.unwrap_or(now).min(now);
    let duration = i64::try_from(request.lookback_hours.get())
        .ok()
        .and_then(TimeDelta::try_hours)
        .ok_or(ActivityError::Window)?;
    let start = now
        .checked_sub_signed(duration)
        .filter(|date| date.year() >= 1)
        .ok_or(ActivityError::Window)?;
    let end = now
        .checked_sub_signed(TimeDelta::minutes(2))
        .filter(|date| date.year() >= 1)
        .ok_or(ActivityError::Window)?;
    Ok(PreviewWindow {
        start: u64::try_from(start.timestamp_millis()).map_err(|_| ActivityError::Transport)?,
        end: u64::try_from(end.timestamp_millis()).map_err(|_| ActivityError::Transport)?,
        offset: request.offset,
    })
}

fn validate_selection(selection: &ActivitySelection) -> Result<(), InvestigationError> {
    if selection.execution_ids.iter().any(|id| {
        ExecutionId::decode(id)
            .is_none_or(|id| !matches!(id.source.as_str(), "traces" | "requests"))
    }) {
        return Err(InvestigationError::InvalidSelection);
    }
    Ok(())
}

async fn preview<R: SessionRepository, D: LensRepository, S: ActivityReader>(
    State(app): State<Arc<App<R, D, S>>>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Json<Public<Sample>>, ActivityError> {
    let parsed = body_validation::parse_body(&body, &headers)?;
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let request: Preview = body_validation::request(
        parsed,
        body_validation::Model::Activity(validation::Model::Preview),
    )
    .map_err(|error| match error {
        crate::error::DatasetError::Decode(_) => ActivityError::Transport,
        error => ActivityError::Request(error),
    })?;
    validate_selection(&request.selection)?;
    let window = window(&request, Utc::now())?;
    let sources = app.sources.as_ref().ok_or(ActivityError::TracingDisabled)?;
    Ok(Json(Public(
        sources
            .preview(&scope(&identity)?, &request.selection, window)
            .await?,
    )))
}

fn content_query(query: Option<&str>) -> Result<(String, u32), ActivityError> {
    let values: std::collections::BTreeMap<_, _> =
        url::form_urlencoded::parse(query.unwrap_or_default().as_bytes()).collect();
    let cursor = values
        .get("cursor")
        .map(|s| s.to_string())
        .unwrap_or_default();
    let Some(offset) = values.get("offset") else {
        return Ok((cursor, 0));
    };
    let value = json!(offset);
    let parsed = body_validation::integer(&value, &[json!("query"), json!("offset")], true)
        .map_err(|error| crate::error::DatasetError::Validation(vec![error]))?;
    let offset = parsed
        .exact()
        .and_then(|offset| u32::try_from(offset).ok())
        .filter(|offset| *offset < u32::MAX)
        .ok_or(ActivityError::Transport)?;
    Ok((cursor, offset))
}

async fn content<R: SessionRepository, D: LensRepository, S: ActivityReader>(
    State(app): State<Arc<App<R, D, S>>>,
    Path((lens_id, execution_id)): Path<(String, String)>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<Public<ExecutionContent>>, ActivityError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let (cursor, offset) = content_query(query.as_deref())?;
    let scope = scope(&identity)?;
    let lens = app
        .repository
        .get(&lens_id)
        .await
        .map_err(InvestigationError::from)?
        .filter(|lens| can_access(&scope, &lens.scope))
        .ok_or(InvestigationError::NotFound)?;
    let id = ExecutionId::decode(&execution_id).ok_or(ActivityError::ExecutionNotFound)?;
    if !lens.scope.all_teams && id.team_id != lens.scope.team_id {
        return Err(ActivityError::ExecutionNotFound);
    }
    let source = match id.source.as_str() {
        "traces" => ExecutionSource::Traces,
        "requests" => ExecutionSource::Requests,
        _ => return Err(ActivityError::ExecutionNotFound),
    };
    let execution = Execution {
        id: execution_id,
        source,
        name: id.trace_id.clone(),
        trace_id: id.trace_id,
        trace_ref: id.trace_ref,
        team_id: id.team_id,
        start_time: String::new(),
        span_count: 1,
        root_seen: source == ExecutionSource::Requests,
        service: String::new(),
        metadata: Vec::new(),
    };
    let sources = app.sources.as_ref().ok_or(ActivityError::TracingDisabled)?;
    Ok(Json(Public(
        sources
            .content(&lens.scope, &execution, &cursor, offset)
            .await?,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::current(None, "2026-01-03T00:00:00Z")]
    #[case::past(Some("2026-01-02T00:00:00Z"), "2026-01-02T00:00:00Z")]
    #[case::future(Some("2026-01-04T00:00:00Z"), "2026-01-03T00:00:00Z")]
    fn preview_windows_preserve_the_lookback_and_ingestion_delay(
        #[case] as_of: Option<&str>,
        #[case] end: &str,
    ) {
        let now = "2026-01-03T00:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let expected = end.parse::<DateTime<Utc>>().unwrap();
        let preview: Preview = serde_json::from_value(
            json!({"selection":{},"as_of":as_of,"lookback_hours":48,"offset":123}),
        )
        .unwrap();
        let window = window(&preview, now).unwrap();
        assert_eq!(
            window.start,
            (expected - TimeDelta::hours(48)).timestamp_millis() as u64
        );
        assert_eq!(
            window.end,
            (expected - TimeDelta::minutes(2)).timestamp_millis() as u64
        );
        assert_eq!(window.offset, 123);
    }

    #[rstest]
    #[case::calendar("0001-01-01T00:00:00Z", true)]
    #[case::transport("1960-01-01T00:00:00Z", false)]
    fn preview_rejects_windows_outside_the_existing_transport(
        #[case] as_of: &str,
        #[case] calendar: bool,
    ) {
        let preview: Preview =
            serde_json::from_value(json!({"selection":{},"as_of":as_of})).unwrap();
        let error = window(&preview, Utc::now()).err().unwrap();
        assert_eq!(matches!(error, ActivityError::Window), calendar);
        assert_eq!(matches!(error, ActivityError::Transport), !calendar);
    }
}
