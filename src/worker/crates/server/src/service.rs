use std::sync::Arc;

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, Method},
    routing::get,
};
use lens_auth::{Authentication, SessionRepository};
use serde::Serialize;

use crate::{auth, error::SessionError};

#[derive(Clone, Debug, Default, Serialize)]
pub struct ServiceStatus {
    pub storage_ready: bool,
    pub credentials_ready: bool,
    pub release: String,
    pub protocol_version: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ServiceConnection {
    pub url: String,
    pub connected: bool,
    pub status: ServiceStatus,
    pub configured: bool,
    pub release: String,
}

pub trait ServiceProvider: Send + Sync {
    fn connection(&self) -> ServiceConnection;
}

struct App<R, S> {
    authentication: Arc<Authentication<R>>,
    service: S,
}

pub fn router<R: SessionRepository + 'static, S: ServiceProvider + 'static>(
    authentication: Arc<Authentication<R>>,
    service: S,
) -> Router {
    Router::new()
        .route("/lens/service", get(connection::<R, S>))
        .layer(axum::middleware::from_fn(
            crate::tracing::redirect_trailing_slash,
        ))
        .with_state(Arc::new(App {
            authentication,
            service,
        }))
}

async fn connection<R: SessionRepository, S: ServiceProvider>(
    State(app): State<Arc<App<R, S>>>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<ServiceConnection>, SessionError> {
    auth::identity(&app.authentication, &headers, &method).await?;
    Ok(Json(app.service.connection()))
}
