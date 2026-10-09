use crate::routing::PublicRoutes;
use crate::{auth, error::InvestigationError};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, Method},
    routing::get,
};
use lens_auth::{Authentication, SessionRepository};
use lens_contract::{activity::AnalysisModelInfo, auth::Role};
use serde::Serialize;
use std::sync::Arc;

struct App<R> {
    authentication: Arc<Authentication<R>>,
    models: Vec<AnalysisModelInfo>,
}

#[derive(Serialize)]
struct ModelList<T> {
    data: Vec<T>,
}

#[derive(Serialize)]
struct Model {
    id: String,
}

pub fn router<R: SessionRepository + 'static>(
    authentication: Arc<Authentication<R>>,
    configured: Vec<AnalysisModelInfo>,
) -> Router {
    Router::new()
        .public_route("/models", get(models::<R>))
        .public_route("/lens/models", get(models::<R>))
        .public_route("/v1/models", get(models::<R>))
        .public_route("/model_group/info", get(groups::<R>))
        .public_route("/lens/model_group/info", get(groups::<R>))
        .layer(axum::middleware::from_fn(
            crate::routing::redirect_trailing_slash,
        ))
        .with_state(Arc::new(App {
            authentication,
            models: configured,
        }))
}

async fn authorize<R: SessionRepository>(
    app: &App<R>,
    headers: &HeaderMap,
    method: &Method,
) -> Result<(), InvestigationError> {
    let identity = auth::identity(&app.authentication, headers, method).await?;
    if !matches!(
        identity.user_role,
        Role::ProxyAdmin | Role::ProxyAdminViewer
    ) {
        return Err(InvestigationError::ForbiddenRead);
    }
    Ok(())
}

async fn models<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<ModelList<Model>>, InvestigationError> {
    authorize(&app, &headers, &method).await?;
    Ok(Json(ModelList {
        data: app
            .models
            .iter()
            .map(|model| Model {
                id: model.model_group.clone(),
            })
            .collect(),
    }))
}

async fn groups<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<ModelList<AnalysisModelInfo>>, InvestigationError> {
    authorize(&app, &headers, &method).await?;
    Ok(Json(ModelList {
        data: app.models.clone(),
    }))
}
