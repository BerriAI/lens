use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use lens_signals::{DecisionRequest, Decisions, DecisionsError};
use litellm_model_catalog::{Catalog, Mode};
use serde_json::{Value, json};
use url::Url;

use crate::{Deployment, Error, Provider, TransportLimits};

pub struct EvaluationModels {
    models: BTreeMap<String, Group>,
    client: reqwest::Client,
    limits: TransportLimits,
    gateway: Option<lens_inference::GatewayIdentity>,
}

struct Group {
    deployments: Vec<Resolved>,
    next: AtomicUsize,
}

struct Resolved {
    config: Deployment,
    model: String,
    endpoint: Url,
}

fn resolve(config: Deployment, catalog: &Catalog) -> Result<Resolved, Error> {
    if config.name.trim().is_empty() || config.model.trim().is_empty() {
        return Err(Error::MissingModel);
    }
    if config.provider != Provider::StrandsDecider
        && config
            .api_key
            .as_ref()
            .is_none_or(|key| key.0.trim().is_empty())
    {
        return Err(Error::MissingCredential { model: config.name });
    }
    if catalog.lookup(&config.model).is_some_and(|entry| {
        entry
            .entry
            .info()
            .mode
            .is_some_and(|mode| mode != Mode::Evaluation)
    }) {
        return Err(Error::ModelSurface {
            model: config.model,
        });
    }
    let base = match &config.api_base {
        Some(base) => base.clone(),
        None => Url::parse(match config.provider {
            Provider::Typesafe => "https://api.typesafe.ai",
            Provider::Perplexity => "https://api.perplexity.ai",
            Provider::OpenRouter => "https://openrouter.ai/api",
            Provider::Cloudflare | Provider::StrandsDecider | Provider::DecisionsCompatible => {
                return Err(Error::Endpoint);
            }
        })
        .map_err(|_| Error::Endpoint)?,
    };
    if !matches!(base.scheme(), "http" | "https")
        || base.host_str().is_none()
        || !base.username().is_empty()
        || base.password().is_some()
        || base.query().is_some()
        || base.fragment().is_some()
    {
        return Err(Error::Endpoint);
    }
    let model = if config.provider == Provider::DecisionsCompatible {
        config.model.as_str()
    } else {
        config
            .model
            .strip_prefix(&format!("{}/", config.provider.name()))
            .unwrap_or(&config.model)
    };
    let base = base.as_str().trim_end_matches('/');
    let (endpoint, model) = if config.provider == Provider::Cloudflare {
        let canonical = if model.starts_with("@cf/") {
            model.into()
        } else {
            format!("@cf/cloudflare/{model}")
        };
        let base = base.strip_suffix("/ai/v1").unwrap_or(base);
        let endpoint = if base.ends_with("/ai/run") {
            format!("{base}/{canonical}")
        } else {
            format!("{base}/ai/run/{canonical}")
        };
        (
            endpoint,
            canonical
                .rsplit('/')
                .next()
                .unwrap_or(&canonical)
                .to_owned(),
        )
    } else {
        let path = match config.provider {
            Provider::Perplexity | Provider::DecisionsCompatible => "/v1/decisions",
            Provider::OpenRouter => "/alpha/decisions",
            _ => "/v1/systemone",
        };
        (
            format!("{}{path}", base.strip_suffix("/v1").unwrap_or(base)),
            model.to_owned(),
        )
    };
    Ok(Resolved {
        config,
        model,
        endpoint: Url::parse(&endpoint).map_err(|_| Error::Endpoint)?,
    })
}

impl EvaluationModels {
    pub fn new(
        catalog: Arc<Catalog>,
        deployments: Vec<Deployment>,
        limits: TransportLimits,
    ) -> Result<Self, Error> {
        if limits.timeout.is_zero() || limits.max_response_bytes == 0 {
            return Err(Error::Limits);
        }
        let mut models = BTreeMap::<String, Group>::new();
        for config in deployments {
            let name = config.name.clone();
            let deployment = resolve(config, &catalog)?;
            models
                .entry(name)
                .or_insert_with(|| Group {
                    deployments: Vec::new(),
                    next: AtomicUsize::new(0),
                })
                .deployments
                .push(deployment);
        }
        let config = litellm_http::Resolution::from(&litellm_http::HttpSettings::default()).config;
        let client = reqwest::ClientBuilder::try_from(&config)?
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()
            .map_err(Error::Transport)?;
        Ok(Self {
            models,
            client,
            limits,
            gateway: None,
        })
    }

    pub fn with_gateway(mut self, gateway: Option<lens_inference::GatewayIdentity>) -> Self {
        self.gateway = gateway;
        self
    }

    pub fn models(&self) -> Vec<String> {
        self.models.keys().cloned().collect()
    }

    pub fn model_groups(&self) -> Vec<lens_contract::activity::AnalysisModelInfo> {
        self.models
            .iter()
            .map(|(name, group)| lens_contract::activity::AnalysisModelInfo {
                model_group: name.clone(),
                providers: group
                    .deployments
                    .iter()
                    .map(|deployment| deployment.config.provider.name())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                mode: "evaluation".into(),
                supported_openai_params: None,
            })
            .collect()
    }

    pub async fn evaluate(&self, request: &DecisionRequest) -> Result<Value, Error> {
        let group = self
            .models
            .get(&request.model)
            .ok_or_else(|| Error::UnknownModel {
                model: request.model.clone(),
            })?;
        let deployment = &group.deployments
            [group.next.fetch_add(1, Ordering::Relaxed) % group.deployments.len()];
        tokio::time::timeout(
            request.timeout.min(self.limits.timeout),
            self.send(deployment, request),
        )
        .await
        .map_err(|_| Error::Timeout)?
    }

    async fn send(&self, deployment: &Resolved, request: &DecisionRequest) -> Result<Value, Error> {
        let mut body =
            json!({"model":deployment.model,"state":request.state,"questions":request.questions});
        if deployment.config.provider == Provider::DecisionsCompatible {
            body["metadata"] = json!({"tags":request.tags});
        }
        let builder = self.client.post(deployment.endpoint.clone()).json(&body);
        let builder = match &deployment.config.api_key {
            Some(key) if !key.0.is_empty() => builder.bearer_auth(&key.0),
            _ => builder,
        };
        let marker = self
            .gateway
            .as_ref()
            .map(|gateway| {
                gateway.token(
                    &deployment.endpoint,
                    lens_inference::GatewayPurpose::Signals,
                    chrono::Utc::now(),
                )
            })
            .transpose()?
            .flatten();
        let builder = match marker {
            Some(marker) => builder.header(lens_inference::GATEWAY_HEADER, marker),
            None => builder,
        };
        let mut response = builder.send().await.map_err(Error::Transport)?;
        let status = response.status();
        if response
            .content_length()
            .is_some_and(|length| length > self.limits.max_response_bytes as u64)
        {
            return Err(Error::ResponseLimit);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(Error::Transport)? {
            if chunk.len() > self.limits.max_response_bytes - bytes.len() {
                return Err(Error::ResponseLimit);
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            return Err(Error::Provider {
                status: status.as_u16(),
            });
        }
        let response: Value = serde_json::from_slice(&bytes)?;
        if deployment.config.provider == Provider::Cloudflare
            && response.get("answers").is_none()
            && let Some(result) = response.get("result").filter(|result| result.is_object())
        {
            return Ok(result.clone());
        }
        Ok(response)
    }
}

impl Decisions for EvaluationModels {
    async fn complete(&self, request: &DecisionRequest) -> Result<Value, DecisionsError> {
        self.evaluate(request).await.map_err(|error| match error {
            Error::Provider { status } => DecisionsError::Provider { status },
            Error::Timeout => DecisionsError::Timeout,
            Error::ResponseLimit => DecisionsError::ResponseLimit,
            error => DecisionsError::Unavailable(Box::new(error)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    #[rstest]
    #[case::typesafe(Provider::Typesafe, "https://api.typesafe.ai/v1/systemone")]
    #[case::perplexity(Provider::Perplexity, "https://api.perplexity.ai/v1/decisions")]
    #[case::openrouter(Provider::OpenRouter, "https://openrouter.ai/api/alpha/decisions")]
    fn native_default_endpoints_match_configured_provider(
        #[case] provider: Provider,
        #[case] expected: &str,
    ) {
        let resolved = resolve(
            Deployment {
                name: "signals".into(),
                model: "test-evaluator".into(),
                provider,
                api_base: None,
                api_key: Some(crate::Secret::new("key")),
            },
            &Catalog::parse(
                br#"{"evaluation-fixture":{"mode":"evaluation","litellm_provider":"typesafe"}}"#,
                Default::default(),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(resolved.endpoint.as_str(), expected);
        assert_eq!(resolved.model, "test-evaluator");
    }
}
