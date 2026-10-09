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
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct GatewayStatus {
    pub configured: bool,
    pub connected: bool,
    pub api_base: Option<String>,
    pub evaluation_models: usize,
    pub analysis_models: usize,
    pub error: Option<String>,
    pub last_refreshed: Option<chrono::DateTime<chrono::Utc>>,
}

pub trait ModelCatalog: Clone + Send + Sync + 'static {
    fn model_groups(&self) -> Vec<AnalysisModelInfo>;
    fn gateway(&self) -> GatewayStatus;
    fn refresh(&self) -> impl std::future::Future<Output = GatewayStatus> + Send;
}

impl ModelCatalog for Vec<AnalysisModelInfo> {
    fn model_groups(&self) -> Vec<AnalysisModelInfo> {
        self.clone()
    }
    fn gateway(&self) -> GatewayStatus {
        GatewayStatus::default()
    }
    async fn refresh(&self) -> GatewayStatus {
        self.gateway()
    }
}

struct App<R, M> {
    authentication: Arc<Authentication<R>>,
    models: M,
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
    with_catalog(authentication, configured)
}

pub fn with_catalog<R: SessionRepository + 'static, M: ModelCatalog>(
    authentication: Arc<Authentication<R>>,
    configured: M,
) -> Router {
    Router::new()
        .public_route("/models", get(models::<R, M>))
        .public_route("/lens/models", get(models::<R, M>))
        .public_route("/v1/models", get(models::<R, M>))
        .public_route("/model_group/info", get(groups::<R, M>))
        .public_route("/lens/model_group/info", get(groups::<R, M>))
        .public_route("/lens/gateway", get(gateway::<R, M>))
        .public_route(
            "/lens/gateway/refresh",
            axum::routing::post(refresh::<R, M>),
        )
        .layer(axum::middleware::from_fn(
            crate::routing::redirect_trailing_slash,
        ))
        .with_state(Arc::new(App {
            authentication,
            models: configured,
        }))
}

async fn authorize<R: SessionRepository, M>(
    app: &App<R, M>,
    headers: &HeaderMap,
    method: &Method,
) -> Result<Role, InvestigationError> {
    let identity = auth::identity(&app.authentication, headers, method).await?;
    if !matches!(
        identity.user_role,
        Role::ProxyAdmin | Role::ProxyAdminViewer
    ) {
        return Err(InvestigationError::ForbiddenRead);
    }
    Ok(identity.user_role)
}

async fn models<R: SessionRepository, M: ModelCatalog>(
    State(app): State<Arc<App<R, M>>>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<ModelList<Model>>, InvestigationError> {
    authorize(&app, &headers, &method).await?;
    Ok(Json(ModelList {
        data: app
            .models
            .model_groups()
            .iter()
            .map(|model| Model {
                id: model.model_group.clone(),
            })
            .collect(),
    }))
}

async fn groups<R: SessionRepository, M: ModelCatalog>(
    State(app): State<Arc<App<R, M>>>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<ModelList<AnalysisModelInfo>>, InvestigationError> {
    authorize(&app, &headers, &method).await?;
    Ok(Json(ModelList {
        data: app.models.model_groups(),
    }))
}

async fn gateway<R: SessionRepository, M: ModelCatalog>(
    State(app): State<Arc<App<R, M>>>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<GatewayStatus>, InvestigationError> {
    authorize(&app, &headers, &method).await?;
    Ok(Json(app.models.gateway()))
}

async fn refresh<R: SessionRepository, M: ModelCatalog>(
    State(app): State<Arc<App<R, M>>>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<GatewayStatus>, InvestigationError> {
    if authorize(&app, &headers, &method).await? != Role::ProxyAdmin {
        return Err(InvestigationError::ForbiddenWrite);
    }
    Ok(Json(app.models.refresh().await))
}
