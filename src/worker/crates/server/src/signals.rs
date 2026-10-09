pub(crate) mod validation;

use crate::{
    auth,
    datasets::validation as body_validation,
    error::{InvestigationError, SignalError},
    routing::PublicRoutes,
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, Method},
    routing::{get, post},
};
use lens_auth::{Authentication, SessionRepository};
use lens_contract::{
    auth::Role,
    feedback::TraceFeedbackRequest,
    signals::{SignalConfig, TraceSignals},
};
use lens_signals::{SignalRepository, trace_signals};
use std::{collections::BTreeMap, sync::Arc};

struct App<R, S> {
    authentication: Arc<Authentication<R>>,
    repository: S,
    models: Vec<String>,
}

pub fn router<R, S>(
    authentication: Arc<Authentication<R>>,
    repository: S,
    models: Vec<String>,
) -> Router
where
    R: SessionRepository + 'static,
    S: SignalRepository + 'static,
{
    Router::new()
        .public_route("/lens/signals", get(read::<R, S>).put(save::<R, S>))
        .public_route("/lens/traces/signals", post(traces::<R, S>))
        .layer(DefaultBodyLimit::disable())
        .layer(axum::middleware::from_fn(
            crate::routing::redirect_trailing_slash,
        ))
        .with_state(Arc::new(App {
            authentication,
            repository,
            models,
        }))
}

fn authorize(role: Role, write: bool) -> Result<(), InvestigationError> {
    match (role, write) {
        (Role::ProxyAdmin, _) | (Role::ProxyAdminViewer, false) => Ok(()),
        (_, true) => Err(InvestigationError::ForbiddenWrite),
        (_, false) => Err(InvestigationError::ForbiddenRead),
    }
}

async fn read<R: SessionRepository, S: SignalRepository>(
    State(app): State<Arc<App<R, S>>>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<SignalConfig>, SignalError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    authorize(identity.user_role, false)?;
    Ok(Json(app.repository.get_config().await?))
}

async fn save<R: SessionRepository, S: SignalRepository>(
    State(app): State<Arc<App<R, S>>>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Json<SignalConfig>, SignalError> {
    let parsed = body_validation::parse_body(&body, &headers)?;
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let config: SignalConfig = body_validation::request(
        parsed,
        body_validation::Model::Signals(validation::Model::Config),
    )?;
    authorize(identity.user_role, true)?;
    if !config.model.is_empty() && !app.models.contains(&config.model) {
        return Err(SignalError::Model);
    }
    app.repository.save_config(&config).await?;
    Ok(Json(config))
}

async fn traces<R: SessionRepository, S: SignalRepository>(
    State(app): State<Arc<App<R, S>>>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Json<Vec<TraceSignals>>, SignalError> {
    let parsed = body_validation::parse_body(&body, &headers)?;
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let request: TraceFeedbackRequest = body_validation::request(
        parsed,
        body_validation::Model::Feedback(crate::feedback::validation::Model::Summary),
    )?;
    authorize(identity.user_role, false)?;
    let config = app.repository.get_config().await?;
    let existing = app.repository.traces(&request.traces).await?;
    let rows: BTreeMap<_, _> = existing
        .iter()
        .map(|row| ((row.trace_id.as_str(), row.trace_ref.as_str()), row))
        .collect();
    Ok(Json(
        request
            .traces
            .iter()
            .map(|trace| {
                trace_signals(
                    trace,
                    rows.get(&(trace.trace_id.as_str(), trace.trace_ref.as_str()))
                        .copied(),
                    &config,
                )
            })
            .collect::<Result<_, _>>()?,
    ))
}
