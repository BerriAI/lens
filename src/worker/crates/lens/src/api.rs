use std::sync::{Arc, atomic::Ordering};

use axum::Router;
use lens_auth::ingestion::Ingestion;
use lens_auth::{Authentication, Settings};
use lens_server::datasets::DatasetConfig;
use lens_server::tracing::TraceConfig;
use litellm_storage_clickhouse::{
    datasets::Datasets, evals::EvalStore, ingestion::IngestionKeys, sessions::Sessions,
    state::ClickHouseState,
};
use litellm_traces_clickhouse::evals::EvalTraces;
use tokio::task::JoinHandle;

use crate::{
    Error, State, eval_judge::GatewayJudge, eval_runtime, local_credentials::LocalCredentials,
    storage::TraceApi,
};

pub struct EvalConfig {
    pub public_url: String,
    pub judge: GatewayJudge,
}

pub struct Application {
    pub router: Router,
    pub credentials: Arc<LocalCredentials<IngestionKeys>>,
    pub evals: JoinHandle<()>,
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
    evals: EvalConfig,
) -> Result<(Router, JoinHandle<()>), Error> {
    let application = initialize(
        state,
        settings,
        datasets,
        TraceConfig::default(),
        false,
        evals,
    )
    .await?;
    Ok((application.router, application.evals))
}

pub async fn initialize(
    state: &Arc<State>,
    settings: Settings,
    datasets: DatasetConfig,
    traces: TraceConfig,
    standalone: bool,
    evals: EvalConfig,
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
    let traces_reader = EvalTraces::new(state.storage.client.clone(), connection.reader().clone());
    let eval_task = eval_runtime::start(
        EvalStore::new(store.clone()),
        traces_reader.clone(),
        evals.judge,
    );
    let (eval_api, eval_cases) = lens_server::evals::split_router_with_traces(
        authentication.clone(),
        store.clone(),
        evals.public_url,
        traces_reader,
    );
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
        ))
        .merge(eval_api);
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
        router: lens_server::evals::with_contract_cases(router, eval_cases),
        credentials,
        evals: eval_task,
        authentication,
    })
}
