use crate::{Deployment, Error, Provider, TransportLimits, catalog, protocol};
use lens_contract::worker::{ModelMessage, ModelRequest, ModelResult};
use lens_inference::{DeploymentEstimate, Usage, UsageEnvelope};
use litellm_cost::{PromptConvention, Request, ServiceTier, ThresholdPolicy};
use litellm_model_catalog::Catalog;
use litellm_token_counter::CountableRequest;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

pub struct AnalysisModels {
    models: BTreeMap<String, DeploymentGroup>,
    client: reqwest::Client,
    limits: TransportLimits,
    gateway: Option<lens_inference::GatewayIdentity>,
}

struct DeploymentGroup {
    deployments: Vec<Arc<catalog::ResolvedDeployment>>,
    next: AtomicUsize,
}

pub struct PreparedAnalysis {
    pub estimate: f64,
    pub context_exceeded: bool,
    request: Option<PreparedRequest>,
}

struct PreparedRequest {
    deployment: Arc<catalog::ResolvedDeployment>,
    body: Value,
    custom_charge: bool,
}

pub struct AnalysisCompletion {
    pub result: ModelResult,
    pub usage: UsageEnvelope,
}

impl AnalysisModels {
    pub fn new(
        catalog: Arc<Catalog>,
        deployments: Vec<Deployment>,
        limits: TransportLimits,
    ) -> Result<Self, Error> {
        if limits.timeout.is_zero() || limits.max_response_bytes == 0 {
            return Err(Error::TransportLimits);
        }
        let mut models = BTreeMap::<String, DeploymentGroup>::new();
        for config in deployments {
            let name = config.name.clone();
            let deployment = Arc::new(catalog::resolve(config, &catalog)?);
            models
                .entry(name)
                .or_insert_with(|| DeploymentGroup {
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
                    .map(|deployment| match deployment.config.provider {
                        Provider::OpenAi => "openai",
                        Provider::Anthropic => "anthropic",
                        Provider::OpenAiCompatible => "openai_compatible",
                    })
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                mode: "chat".into(),
                supported_openai_params: None,
            })
            .collect()
    }

    pub async fn prepare(
        &self,
        alias: &str,
        body: &ModelRequest,
    ) -> Result<PreparedAnalysis, Error> {
        let group = self.models.get(alias).ok_or_else(|| Error::UnknownModel {
            model: alias.into(),
        })?;
        let deployments = group.deployments.clone();
        let messages = lens_inference::request_messages(body)?;
        let count_messages = messages.clone();
        let counts =
            tokio::task::spawn_blocking(move || prompt_counts(&deployments, &count_messages))
                .await
                .map_err(Error::TokenizerTask)??;
        let candidates: Vec<_> = group
            .deployments
            .iter()
            .zip(&counts)
            .filter(|(deployment, count)| {
                !lens_inference::deployment_exceeds_context(
                    **count,
                    deployment.config.capacity.max_input_tokens,
                    deployment.catalog_capacity.max_input_tokens,
                )
            })
            .collect();
        if candidates.is_empty() {
            return Ok(PreparedAnalysis {
                estimate: 0.0,
                context_exceeded: true,
                request: None,
            });
        }
        let estimates = group
            .deployments
            .iter()
            .zip(counts.iter().copied())
            .map(|(deployment, prompt_tokens)| {
                let output_tokens = lens_inference::output_tokens(
                    &deployment.config.model,
                    &deployment.config.output_limits,
                    deployment.catalog_capacity,
                    Some(prompt_tokens),
                )?;
                Ok(DeploymentEstimate {
                    prices: deployment.prices,
                    prompt_tokens,
                    output_tokens,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let output = estimates
            .iter()
            .map(|estimate| estimate.output_tokens)
            .min()
            .ok_or(lens_inference::Error::ModelUnavailable)?;
        let boundaries = lens_inference::cache_injection_points(body);
        let estimate = lens_inference::quote(&estimates, !boundaries.is_empty())?;
        let selected = group.next.fetch_add(1, Ordering::Relaxed) % candidates.len();
        let deployment = candidates[selected].0.clone();
        let body = protocol::request(&deployment, &messages, &boundaries, output)?;
        let custom_charge = group.deployments.len() == 1
            && deployment
                .config
                .input_cost_per_token
                .zip(deployment.config.output_cost_per_token)
                .is_some();
        Ok(PreparedAnalysis {
            estimate,
            context_exceeded: false,
            request: Some(PreparedRequest {
                deployment,
                body,
                custom_charge,
            }),
        })
    }

    pub async fn complete(&self, prepared: &PreparedAnalysis) -> Result<AnalysisCompletion, Error> {
        let Some(request) = &prepared.request else {
            return Ok(context_completion());
        };
        tokio::time::timeout(self.limits.timeout, self.send(request, prepared.estimate))
            .await
            .map_err(|_| Error::Timeout)?
    }

    async fn send(
        &self,
        request: &PreparedRequest,
        estimate: f64,
    ) -> Result<AnalysisCompletion, Error> {
        let deployment = &request.deployment;
        let builder = self
            .client
            .post(deployment.endpoint.clone())
            .json(&request.body);
        let builder = match deployment.config.provider {
            Provider::Anthropic => builder
                .header("x-api-key", deployment.config.api_key.expose())
                .header("anthropic-version", "2023-06-01"),
            Provider::OpenAi | Provider::OpenAiCompatible => {
                builder.bearer_auth(deployment.config.api_key.expose())
            }
        };
        let marker = self
            .gateway
            .as_ref()
            .map(|gateway| {
                gateway.token(
                    &deployment.endpoint,
                    lens_inference::GatewayPurpose::Analysis,
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
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());
        if response
            .content_length()
            .is_some_and(|length| length > self.limits.max_response_bytes as u64)
        {
            return Err(Error::ResponseLimit);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(Error::Transport)? {
            if chunk.len() > self.limits.max_response_bytes - body.len() {
                return Err(Error::ResponseLimit);
            }
            body.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            if status.as_u16() == 400
                && protocol::context_failure(deployment.config.provider, &body)
            {
                return Ok(context_completion());
            }
            return Err(Error::Provider {
                status: status.as_u16(),
                retry_after,
            });
        }
        let completion = protocol::response(deployment.config.provider, &deployment.model, &body)?;
        let usage = normalized_usage(completion.usage)?;
        let actual = deployment
            .pricing
            .calculate(&Request {
                usage: completion.usage,
                service_tier: ServiceTier::Standard,
                threshold_policy: ThresholdPolicy::Exclusive,
                region_multiplier: None,
                billed_at_utc_minute: None,
            })
            .map_err(Error::Cost)?
            .total();
        let custom = request.custom_charge.then_some(&deployment.prices);
        let cost = lens_inference::completion_charge(custom, usage, actual, estimate);
        Ok(AnalysisCompletion {
            result: lens_inference::model_result(
                completion.content.as_deref(),
                cost,
                completion.finish_reason.as_deref(),
            ),
            usage: UsageEnvelope {
                model: Some(completion.model),
                usage: Some(usage),
            },
        })
    }
}

fn prompt_counts(
    deployments: &[Arc<catalog::ResolvedDeployment>],
    messages: &[ModelMessage],
) -> Result<Vec<u64>, Error> {
    deployments
        .iter()
        .map(|deployment| {
            let request: CountableRequest = serde_json::from_value(
                serde_json::json!({"model":deployment.config.model,"messages":messages}),
            )?;
            let count = deployment.counter.count_request(&request)?.input_tokens;
            let legacy = if deployment.legacy_accounting {
                messages.len()
            } else {
                0
            };
            let count = count.checked_add(legacy).ok_or(Error::TokenCount)?;
            u64::try_from(count).map_err(|_| Error::TokenCount)
        })
        .collect()
}

fn normalized_usage(usage: litellm_cost::Usage) -> Result<Usage, Error> {
    let prompt = match usage.prompt_convention {
        PromptConvention::IncludesCache => usage.prompt_tokens,
        PromptConvention::ExcludesCache => usage
            .prompt_tokens
            .checked_add(usage.cache_read_tokens)
            .and_then(|tokens| tokens.checked_add(usage.cache_write_tokens))
            .ok_or(Error::TokenCount)?,
    };
    Ok(Usage {
        prompt_tokens: Some(i64::try_from(prompt).map_err(|_| Error::TokenCount)?),
        completion_tokens: Some(
            i64::try_from(usage.completion_tokens).map_err(|_| Error::TokenCount)?,
        ),
    })
}

fn context_completion() -> AnalysisCompletion {
    AnalysisCompletion {
        result: ModelResult {
            content: String::new(),
            cost: 0.0,
            context_exceeded: true,
            finish_reason: None,
        },
        usage: UsageEnvelope::default(),
    }
}
