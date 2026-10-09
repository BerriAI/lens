use std::sync::{Arc, atomic::Ordering};

use axum::Router;
use lens_auth::ingestion::Ingestion;
use lens_auth::{Authentication, Settings};
use lens_server::datasets::DatasetConfig;
use lens_server::tracing::TraceConfig;
use litellm_storage_clickhouse::{
    datasets::Datasets, ingestion::IngestionKeys, sessions::Sessions, state::ClickHouseState,
};
use litellm_traces_clickhouse::evals::EvalTraces;

use crate::{Error, State, local_credentials::LocalCredentials, storage::TraceApi};

pub struct Application {
    pub router: Router,
    pub credentials: Arc<LocalCredentials<IngestionKeys>>,
    authentication: Arc<Authentication<Sessions>>,
}

impl Application {
    pub fn eval_store(&self) -> litellm_storage_clickhouse::evals::EvalStore {
        litellm_storage_clickhouse::evals::EvalStore::new(self.authentication.sessions.0.clone())
    }

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
    public_url: String,
) -> Result<Router, Error> {
    Ok(initialize(
        state,
        settings,
        datasets,
        TraceConfig::default(),
        false,
        public_url,
    )
    .await?
    .router)
}

pub async fn initialize(
    state: &Arc<State>,
    settings: Settings,
    datasets: DatasetConfig,
    traces: TraceConfig,
    standalone: bool,
    public_url: String,
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
    let router = router_with_evals(
        state,
        authentication.clone(),
        datasets,
        public_url,
        EvalTraces::new(state.storage.client.clone(), connection.reader().clone()),
    )
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

pub fn router_with_evals(
    state: &State,
    authentication: Arc<Authentication<Sessions>>,
    datasets: DatasetConfig,
    public_url: String,
    traces: EvalTraces,
) -> Router {
    let store = authentication.sessions.0.clone();
    lens_server::sessions::router_with_auth(authentication.clone())
        .merge(lens_server::datasets::router_with_evals(
            authentication.clone(),
            Datasets(store.clone()),
            state.storage.dataset_reader(store.clone()),
            datasets,
            store.clone(),
            Some(traces.clone()),
        ))
        .merge(lens_server::evals::router_without_cases(
            authentication,
            store,
            public_url,
            Some(traces),
        ))
}
