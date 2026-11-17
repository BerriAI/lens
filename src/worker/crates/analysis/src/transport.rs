use crate::{Deployment, Error, PrepareOptions, Provider, TransportLimits, catalog, protocol};
use lens_contract::worker::{ModelMessage, ModelRequest, ModelResult};
use lens_inference::{DeploymentEstimate, Usage, UsageEnvelope};
use litellm_cost::{PromptConvention, Request, ServiceTier, ThresholdPolicy};
use litellm_model_catalog::Catalog;
use litellm_token_counter::CountableRequest;
use serde::Serialize;
use serde_json::Value;
use std::{
    borrow::Cow,
    collections::BTreeMap,
    num::NonZeroU64,
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
    pub output_tokens: u64,
    output_ceiling: u64,
    request: Option<PreparedRequest>,
}

impl PreparedAnalysis {
    pub fn larger_output(&self) -> Option<NonZeroU64> {
        if self.output_tokens >= self.output_ceiling {
            return None;
        }
        NonZeroU64::new(
            self.output_tokens
                .saturating_mul(OUTPUT_GROWTH)
                .min(self.output_ceiling),
        )
    }
}

const OUTPUT_GROWTH: u64 = 4;

struct PreparedRequest {
    deployment: Arc<catalog::ResolvedDeployment>,
    body: Value,
    plain: Option<Value>,
    quick: Option<Value>,
    custom_charge: bool,
}

struct Reply {
    status: reqwest::StatusCode,
    headers: reqwest::header::HeaderMap,
    body: Vec<u8>,
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
        self.prepare_with(alias, body, PrepareOptions::default())
            .await
    }

    pub async fn prepare_with(
        &self,
        alias: &str,
        body: &ModelRequest,
        options: PrepareOptions<'_>,
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
                output_tokens: 0,
                output_ceiling: 0,
                request: None,
            });
        }
        let allowances = group
            .deployments
            .iter()
            .zip(counts.iter().copied())
            .map(|(deployment, prompt_tokens)| {
                let configured = lens_inference::output_tokens(
                    &deployment.config.model,
                    &deployment.config.output_limits,
                    deployment.catalog_capacity,
                    Some(prompt_tokens),
                )?;
                let ceiling = lens_inference::output_tokens(
                    &deployment.config.model,
                    &lens_inference::OutputLimits {
                        max_output_tokens: deployment.config.capacity.max_output_tokens,
                        ..Default::default()
                    },
                    deployment.catalog_capacity,
                    Some(prompt_tokens),
                )
                .map_or(configured, |capacity| capacity.max(configured));
                let output_tokens = options.minimum_output_tokens.map_or(configured, |minimum| {
                    configured.max(minimum.get()).min(ceiling)
                });
                Ok((
                    DeploymentEstimate {
                        prices: deployment.prices,
                        prompt_tokens,
                        output_tokens,
                    },
                    ceiling,
                ))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let (estimates, ceilings): (Vec<_>, Vec<_>) = allowances.into_iter().unzip();
        let output = estimates
            .iter()
            .map(|estimate| estimate.output_tokens)
            .min()
            .ok_or(lens_inference::Error::ModelUnavailable)?;
        let output_ceiling = ceilings.into_iter().min().unwrap_or(output);
        let boundaries = lens_inference::cache_injection_points(body);
        let estimate = lens_inference::quote(&estimates, !boundaries.is_empty())?;
        let selected = group.next.fetch_add(1, Ordering::Relaxed) % candidates.len();
        let deployment = candidates[selected].0.clone();
        let provider = deployment.config.provider;
        let plain_only = deployment.plain_requests.load(Ordering::Relaxed);
        let reduced = deployment.reduced_reasoning.load(Ordering::Relaxed);
        let schema = options
            .response_schema
            .as_ref()
            .filter(|_| !plain_only)
            .and_then(protocol::strict_schema);
        let paced = |body: Value| {
            reduced
                .then(|| protocol::low_reasoning(provider, &body))
                .flatten()
                .unwrap_or(body)
        };
        let unmarked: &[usize] = if deployment.cache_marks == catalog::CacheMarks::Forwarded {
            &[]
        } else {
            &boundaries
        };
        let body = paced(protocol::request(
            &deployment,
            &messages,
            if plain_only { unmarked } else { &boundaries },
            output,
            schema.as_ref(),
            !plain_only,
        )?);
        let plain = (!plain_only)
            .then(|| protocol::request(&deployment, &messages, unmarked, output, None, false))
            .transpose()?
            .map(paced);
        let quick = (!reduced)
            .then(|| protocol::low_reasoning(provider, &body))
            .flatten();
        let custom_charge = group.deployments.len() == 1
            && deployment
                .config
                .input_cost_per_token
                .zip(deployment.config.output_cost_per_token)
                .is_some();
        Ok(PreparedAnalysis {
            estimate,
            context_exceeded: false,
            output_tokens: output,
            output_ceiling,
            request: Some(PreparedRequest {
                deployment,
                body,
                plain,
                quick,
                custom_charge,
            }),
        })
    }

    pub async fn complete(&self, prepared: &PreparedAnalysis) -> Result<AnalysisCompletion, Error> {
        self.complete_request(prepared, None).await
    }

    pub async fn complete_with_gateway_metadata(
        &self,
        prepared: &PreparedAnalysis,
        metadata: &impl Serialize,
    ) -> Result<AnalysisCompletion, Error> {
        self.complete_request(prepared, Some(serde_json::to_value(metadata)?))
            .await
    }

    async fn complete_request(
        &self,
        prepared: &PreparedAnalysis,
        metadata: Option<Value>,
    ) -> Result<AnalysisCompletion, Error> {
        let Some(request) = &prepared.request else {
            return Ok(context_completion());
        };
        tokio::time::timeout(
            self.limits.timeout,
            self.send(request, prepared.estimate, metadata),
        )
        .await
        .map_err(|_| Error::Timeout)?
    }

    async fn exchange(
        &self,
        deployment: &catalog::ResolvedDeployment,
        body: &Value,
        marker: Option<&str>,
        metadata: Option<&Value>,
    ) -> Result<Reply, Error> {
        let mut body = Cow::Borrowed(body);
        if marker.is_some()
            && let Some(metadata) = metadata
        {
            body.to_mut()["metadata"] = metadata.clone();
        }
        let builder = self
            .client
            .post(deployment.endpoint.clone())
            .json(body.as_ref());
        let builder = match deployment.config.provider {
            Provider::Anthropic => builder
                .header("x-api-key", deployment.config.api_key.expose())
                .header("anthropic-version", "2023-06-01"),
            Provider::OpenAi | Provider::OpenAiCompatible => {
                builder.bearer_auth(deployment.config.api_key.expose())
            }
        };
        let builder = match marker {
            Some(marker) => builder.header(lens_inference::GATEWAY_HEADER, marker),
            None => builder,
        };
        let mut response = builder.send().await.map_err(Error::Transport)?;
        let status = response.status();
        let headers = response.headers().clone();
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
        Ok(Reply {
            status,
            headers,
            body,
        })
    }

    async fn send(
        &self,
        request: &PreparedRequest,
        estimate: f64,
        metadata: Option<Value>,
    ) -> Result<AnalysisCompletion, Error> {
        let deployment = &request.deployment;
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
        let provider = deployment.config.provider;
        let first = self
            .exchange(
                deployment,
                &request.body,
                marker.as_deref(),
                metadata.as_ref(),
            )
            .await?;
        let refused = matches!(first.status.as_u16(), 400 | 422)
            && !protocol::context_failure(provider, &first.body);
        let timed_out = matches!(first.status.as_u16(), 408 | 504 | 524);
        let reply = match (&request.plain, &request.quick) {
            (Some(plain), _) if refused => {
                let retried = self
                    .exchange(deployment, plain, marker.as_deref(), metadata.as_ref())
                    .await?;
                if retried.status.is_success() {
                    deployment.plain_requests.store(true, Ordering::Relaxed);
                }
                retried
            }
            (_, Some(quick)) if timed_out => {
                let retried = self
                    .exchange(deployment, quick, marker.as_deref(), metadata.as_ref())
                    .await?;
                if retried.status.is_success() {
                    deployment.reduced_reasoning.store(true, Ordering::Relaxed);
                    retried
                } else {
                    first
                }
            }
            _ => first,
        };
        let Reply {
            status,
            headers,
            body,
        } = reply;
        if !status.is_success() {
            if status.as_u16() == 400 && protocol::context_failure(provider, &body) {
                return Ok(context_completion());
            }
            let retry_after = headers
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok());
            return Err(Error::Provider {
                status: status.as_u16(),
                retry_after,
                diagnostic: crate::ProviderDiagnostic::from_response(
                    status.as_u16(),
                    &headers,
                    &body,
                ),
            });
        }
        let streamed = headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream"));
        let completion = match protocol::response(provider, &deployment.model, &body, streamed)? {
            protocol::Outcome::Complete(completion) => completion,
            protocol::Outcome::Failed(failure) if protocol::context_failure(provider, failure) => {
                return Ok(context_completion());
            }
            protocol::Outcome::Failed(failure) => return Err(interrupted(&headers, failure)),
            protocol::Outcome::Cut => return Err(interrupted(&headers, &[])),
        };
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
        let cost = if custom.is_some() && deployment.cache_priced {
            actual
        } else {
            lens_inference::completion_charge(custom, usage, actual, estimate)
        };
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

fn interrupted(headers: &reqwest::header::HeaderMap, failure: &[u8]) -> Error {
    let status = reqwest::StatusCode::BAD_GATEWAY.as_u16();
    Error::Provider {
        status,
        retry_after: None,
        diagnostic: crate::ProviderDiagnostic::from_response(status, headers, failure),
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
