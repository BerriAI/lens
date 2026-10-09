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
use lens_investigations::WorkerRepository;
use sha2::{Digest, Sha256};

pub struct Application {
    pub router: Router,
    pub credentials: Arc<LocalCredentials<IngestionKeys>>,
    pub local_worker: Option<crate::local::LocalControl>,
    pub signals_worker: Option<crate::signals::SignalsWorker>,
    pub evaluations:
        Option<crate::evaluations::Evaluations<litellm_storage_clickhouse::evals::Evals>>,
    authentication: Arc<Authentication<Sessions>>,
}

impl Application {
    pub fn with_evaluations(
        mut self,
        state: Arc<State>,
        public_url: url::Url,
    ) -> Result<Self, Error> {
        let worker = self.local_worker.as_ref().ok_or(Error::Unavailable)?;
        let store = self.authentication.sessions.0.clone();
        let repository = litellm_storage_clickhouse::evals::Evals(store.clone());
        self.router = self.router.merge(lens_server::evals::router(
            self.authentication.clone(),
            Datasets(store),
            repository.clone(),
            lens_server::evals::EvalConfig { public_url },
        ));
        self.evaluations = Some(crate::evaluations::Evaluations::new(
            repository,
            state,
            worker.models.clone(),
        ));
        Ok(self)
    }

    pub async fn with_local(
        mut self,
        state: Arc<State>,
        deployments: Vec<lens_analysis::Deployment>,
        evaluation_deployments: Vec<lens_decisions::Deployment>,
        seed: &str,
        gateway: Option<lens_inference::GatewayIdentity>,
    ) -> Result<Self, Error> {
        let catalog = lens_analysis::bundled_catalog()?;
        let models = Arc::new(
            lens_analysis::AnalysisModels::new(catalog.clone(), deployments, Default::default())?
                .with_gateway(gateway.clone()),
        );
        let evaluation = Arc::new(
            lens_decisions::EvaluationModels::new(
                catalog,
                evaluation_deployments,
                Default::default(),
            )?
            .with_gateway(gateway),
        );
        if evaluation
            .models()
            .iter()
            .any(|alias| models.models().contains(alias))
        {
            return Err(Error::Configuration(
                "Analysis and evaluation model aliases must be different",
            ));
        }
        let signals = crate::signals::SignalsWorker {
            repository: litellm_storage_clickhouse::signals::Signals(
                self.authentication.sessions.0.clone(),
            ),
            sources: crate::SourceReader(state.clone()),
            models: evaluation,
        };
        let repository = litellm_storage_clickhouse::investigations::Investigations(
            self.authentication.sessions.0.clone(),
        );
        repository.initialize().await?;
        let token_hash = format!(
            "{:x}",
            Sha256::digest(format!("lens.local.worker.v1\0{seed}"))
        );
        let worker = lens_contract::investigations::Worker {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Lens".into(),
            scope: lens_contract::investigations::Scope {
                all_teams: true,
                ..Default::default()
            },
            last_seen: chrono::Utc::now(),
            revoked: false,
            analysis_key_id: (!models.models().is_empty()).then(|| token_hash.clone()),
        };
        repository
            .configure_service_worker(&worker, &token_hash)
            .await?;
        let control = crate::local::LocalControl::new(
            repository,
            crate::SourceReader(state),
            models,
            token_hash,
        );
        self.router = self
            .router
            .merge(lens_server::investigations::router(
                self.authentication.clone(),
                control.repository.clone(),
                Arc::new(control.clone()),
            ))
            .merge(lens_server::activity::router(
                self.authentication.clone(),
                control.repository.clone(),
                Some(control.sources.clone()),
            ))
            .merge(lens_server::models::router(
                self.authentication.clone(),
                control
                    .models
                    .model_groups()
                    .into_iter()
                    .chain(signals.models.model_groups())
                    .collect(),
            ))
            .merge(lens_server::signals::router(
                self.authentication.clone(),
                signals.repository.clone(),
                signals.models.models(),
            ));
        self.local_worker = Some(control);
        self.signals_worker = Some(signals);
        Ok(self)
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
                public_contract: 1,
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
        ))
        .merge(lens_server::feedback::router(
            authentication.clone(),
            crate::FeedbackApi(state.clone()),
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
        local_worker: None,
        signals_worker: None,
        evaluations: None,
        authentication,
    })
}
