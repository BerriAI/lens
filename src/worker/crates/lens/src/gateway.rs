mod discovery;

use std::{
    sync::{Arc, RwLock},
    time::Duration,
};

use lens_analysis::AnalysisModels;
use lens_decisions::EvaluationModels;
use lens_server::models::{GatewayStatus, ModelCatalog};
use tokio::sync::Mutex;
use url::Url;

use crate::{Error, error::GatewayError};

#[derive(Clone)]
pub struct GatewayConfig {
    base: Url,
    key: String,
}

impl GatewayConfig {
    pub fn new(base: &str, key: String) -> Result<Self, Error> {
        let mut base: Url = base
            .parse()
            .map_err(|_| Error::Configuration("LENS_GATEWAY_API_BASE"))?;
        if !matches!(base.scheme(), "http" | "https")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(Error::Configuration(
                "LENS_GATEWAY_API_BASE must be an HTTP URL without credentials, query or fragment",
            ));
        }
        if key.trim().is_empty() {
            return Err(Error::Configuration("LENS_GATEWAY_API_KEY"));
        }
        let path = base.path().trim_end_matches('/');
        let path = path.strip_suffix("/v1").unwrap_or(path).to_owned();
        base.set_path(&path);
        Ok(Self { base, key })
    }
}

struct Snapshot {
    analysis: Arc<AnalysisModels>,
    evaluation: Arc<EvaluationModels>,
    status: GatewayStatus,
}

struct Registry {
    snapshot: RwLock<Arc<Snapshot>>,
    refresh: Mutex<()>,
    client: reqwest::Client,
    connection: Option<GatewayConfig>,
    signing: Option<lens_inference::GatewayIdentity>,
    analysis: Vec<lens_analysis::Deployment>,
    evaluation: Vec<lens_decisions::Deployment>,
}

#[derive(Clone)]
pub struct Models(Arc<Registry>);

impl Models {
    pub fn new(
        analysis: Vec<lens_analysis::Deployment>,
        evaluation: Vec<lens_decisions::Deployment>,
        signing: Option<lens_inference::GatewayIdentity>,
        connection: Option<GatewayConfig>,
    ) -> Result<Self, Error> {
        let (analysis_models, evaluation_models) =
            compile(analysis.clone(), evaluation.clone(), signing.clone())?;
        let status = GatewayStatus {
            configured: connection.is_some(),
            api_base: connection
                .as_ref()
                .map(|connection| connection.base.to_string().trim_end_matches('/').to_owned()),
            ..Default::default()
        };
        let config = litellm_http::Resolution::from(&litellm_http::HttpSettings::default()).config;
        let client = reqwest::ClientBuilder::try_from(&config)?
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(Duration::from_secs(10))
            .build()?;
        Ok(Self(Arc::new(Registry {
            snapshot: RwLock::new(Arc::new(Snapshot {
                analysis: analysis_models,
                evaluation: evaluation_models,
                status,
            })),
            refresh: Mutex::new(()),
            client,
            connection,
            signing,
            analysis,
            evaluation,
        })))
    }

    fn snapshot(&self) -> Arc<Snapshot> {
        self.0
            .snapshot
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn analysis(&self) -> Arc<AnalysisModels> {
        self.snapshot().analysis.clone()
    }
    pub fn evaluation(&self) -> Arc<EvaluationModels> {
        self.snapshot().evaluation.clone()
    }

    async fn discover(&self, connection: &GatewayConfig) -> Result<Snapshot, GatewayError> {
        let discovered = discovery::fetch(
            &self.0.client,
            connection,
            &self.0.analysis,
            &self.0.evaluation,
        )
        .await?;
        let mut analysis = self.0.analysis.clone();
        let mut evaluation = self.0.evaluation.clone();
        let analysis_count = discovered.analysis.len();
        let evaluation_count = discovered.evaluation.len();
        analysis.extend(discovered.analysis);
        evaluation.extend(discovered.evaluation);
        let (analysis, evaluation) = compile(analysis, evaluation, self.0.signing.clone())
            .map_err(|_| GatewayError::Models)?;
        let mut status = self.gateway();
        status.connected = true;
        status.analysis_models = analysis_count;
        status.evaluation_models = evaluation_count;
        status.last_refreshed = Some(chrono::Utc::now());
        status.error = (discovered.unavailable > 0).then(|| {
            GatewayError::Metadata {
                count: discovered.unavailable,
            }
            .to_string()
        });
        Ok(Snapshot {
            analysis,
            evaluation,
            status,
        })
    }
}

impl ModelCatalog for Models {
    fn model_groups(&self) -> Vec<lens_contract::activity::AnalysisModelInfo> {
        let snapshot = self.snapshot();
        snapshot
            .analysis
            .model_groups()
            .into_iter()
            .chain(snapshot.evaluation.model_groups())
            .collect()
    }
    fn gateway(&self) -> GatewayStatus {
        self.snapshot().status.clone()
    }
    async fn refresh(&self) -> GatewayStatus {
        let Some(connection) = &self.0.connection else {
            return self.gateway();
        };
        let _refresh = self.0.refresh.lock().await;
        let snapshot = match self.discover(connection).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let previous = self.snapshot();
                let mut status = previous.status.clone();
                status.connected = false;
                status.error = Some(error.to_string());
                Snapshot {
                    analysis: previous.analysis.clone(),
                    evaluation: previous.evaluation.clone(),
                    status,
                }
            }
        };
        let status = snapshot.status.clone();
        *self
            .0
            .snapshot
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::new(snapshot);
        status
    }
}

fn compile(
    analysis: Vec<lens_analysis::Deployment>,
    evaluation: Vec<lens_decisions::Deployment>,
    signing: Option<lens_inference::GatewayIdentity>,
) -> Result<(Arc<AnalysisModels>, Arc<EvaluationModels>), Error> {
    let catalog = lens_analysis::bundled_catalog()?;
    let analysis = Arc::new(
        AnalysisModels::new(catalog.clone(), analysis, Default::default())?
            .with_gateway(signing.clone()),
    );
    let evaluation = Arc::new(
        EvaluationModels::new(catalog, evaluation, Default::default())?.with_gateway(signing),
    );
    if evaluation
        .models()
        .iter()
        .any(|alias| analysis.models().contains(alias))
    {
        return Err(Error::Configuration(
            "Analysis and evaluation model aliases must be different",
        ));
    }
    Ok((analysis, evaluation))
}

#[derive(Clone)]
pub enum AnalysisSource {
    Static(Arc<AnalysisModels>),
    Connected(Models),
}

impl AnalysisSource {
    pub(crate) fn refreshes(&self) -> bool {
        matches!(self, Self::Connected(models) if models.0.connection.is_some())
    }

    pub fn get(&self) -> Arc<AnalysisModels> {
        match self {
            Self::Static(models) => models.clone(),
            Self::Connected(models) => models.analysis(),
        }
    }
}

impl From<Arc<AnalysisModels>> for AnalysisSource {
    fn from(models: Arc<AnalysisModels>) -> Self {
        Self::Static(models)
    }
}
impl From<Models> for AnalysisSource {
    fn from(models: Models) -> Self {
        Self::Connected(models)
    }
}
