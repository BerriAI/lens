use std::sync::{Arc, atomic::Ordering};

use axum::Router;
use lens_auth::ingestion::Ingestion;
use lens_auth::{Authentication, Settings};
use lens_server::datasets::DatasetConfig;
use lens_server::tracing::TraceConfig;
use litellm_storage_clickhouse::{
    datasets::Datasets, ingestion::IngestionKeys, sessions::Sessions, state::ClickHouseState,
};

use crate::{Error, State, local_credentials::LocalCredentials, storage::TraceApi};

pub struct Application {
    pub router: Router,
    pub credentials: Arc<LocalCredentials<IngestionKeys>>,
    authentication: Arc<Authentication<Sessions>>,
}

impl Application {
    pub fn with_service(self, state: Arc<State>, url: String, release: String) -> Self {
        let router = self.router.merge(lens_server::service::router(
            self.authentication.clone(),
            LocalService {
                state,
                url,
                release,
            },
        ));
        Self { router, ..self }
    }
}

struct LocalService {
    state: Arc<State>,
    url: String,
    release: String,
}

impl lens_server::service::ServiceProvider for LocalService {
    fn connection(&self) -> lens_server::service::ServiceConnection {
        lens_server::service::ServiceConnection {
            url: self.url.clone(),
            connected: true,
            configured: true,
            release: self.release.clone(),
            status: lens_server::service::ServiceStatus {
                storage_ready: self.state.schema_ready.load(Ordering::Acquire),
                credentials_ready: self.state.credentials.ready(),
                release: self.release.clone(),
                protocol_version: crate::wire::PROTOCOL_VERSION,
            },
        }
    }
}

pub async fn router(
    state: &Arc<State>,
    settings: Settings,
    datasets: DatasetConfig,
) -> Result<Router, Error> {
    Ok(
        initialize(state, settings, datasets, TraceConfig::default(), false)
            .await?
            .router,
    )
}

pub async fn initialize(
    state: &Arc<State>,
    settings: Settings,
    datasets: DatasetConfig,
    traces: TraceConfig,
    standalone: bool,
) -> Result<Application, Error> {
    state.storage.ensure_schema().await?;
    let connection = state.storage.config.storage();
    let store = ClickHouseState::new(state.storage.client.clone(), connection.reader().clone());
    store
        .initialize(&format!("/lens/{}", connection.database()))
        .await?;
    let authentication = Arc::new(Authentication {
        settings,
        sessions: Sessions(store.clone()),
    });
    let ingestion = Ingestion(IngestionKeys(store.clone()));
    let credentials = Arc::new(LocalCredentials::new(
        ingestion.clone(),
        state.credentials.clone(),
    ));
    let router = lens_server::sessions::router_with_auth(authentication.clone())
        .merge(lens_server::datasets::router(
            authentication.clone(),
            Datasets(store.clone()),
            state.storage.dataset_reader(store),
            datasets,
        ))
        .merge(lens_server::tracing::router(
            authentication.clone(),
            TraceApi(state.clone()),
            traces,
        ));
    let router = if standalone {
        router.merge(lens_server::ingestion::router(
            authentication.clone(),
            ingestion,
            credentials.clone(),
        ))
    } else {
        router
    };
    state.schema_ready.store(true, Ordering::Release);
    Ok(Application {
        router,
        credentials,
        authentication,
    })
}
