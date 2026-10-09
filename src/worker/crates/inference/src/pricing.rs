use crate::Error;
use serde::{Deserialize, Deserializer};
use std::num::NonZeroU64;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
pub struct Prices {
    pub input_cost_per_token: f64,
    pub output_cost_per_token: f64,
    #[serde(default, deserialize_with = "tier_rate")]
    pub input_cost_per_token_above_200k_tokens: f64,
    #[serde(default, deserialize_with = "tier_rate")]
    pub output_cost_per_token_above_200k_tokens: f64,
    #[serde(default, deserialize_with = "tier_rate")]
    pub input_cost_per_token_above_128k_tokens: f64,
    #[serde(default, deserialize_with = "tier_rate")]
    pub output_cost_per_token_above_128k_tokens: f64,
    #[serde(default, deserialize_with = "tier_rate")]
    pub input_cost_per_token_above_272k_tokens: f64,
    #[serde(default, deserialize_with = "tier_rate")]
    pub output_cost_per_token_above_272k_tokens: f64,
    #[serde(default, deserialize_with = "tier_rate")]
    pub cache_creation_input_token_cost: f64,
    #[serde(default, deserialize_with = "tier_rate")]
    pub cache_creation_input_token_cost_above_200k_tokens: f64,
    #[serde(default, deserialize_with = "tier_rate")]
    pub cache_creation_input_token_cost_above_272k_tokens: f64,
}

fn tier_rate<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
    Ok(Option::<f64>::deserialize(deserializer)?.unwrap_or(0.0))
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct ModelCapacity {
    pub max_input_tokens: Option<NonZeroU64>,
    pub max_output_tokens: Option<NonZeroU64>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct OutputLimits {
    pub max_completion_tokens: Option<NonZeroU64>,
    pub max_tokens: Option<NonZeroU64>,
    pub max_output_tokens: Option<NonZeroU64>,
}

#[derive(Clone, Copy, Debug)]
pub struct DeploymentEstimate {
    pub prices: Prices,
    pub prompt_tokens: u64,
    pub output_tokens: u64,
}

pub fn deployment_prices(
    model: &str,
    custom_input: Option<f64>,
    custom_output: Option<f64>,
    catalog: Option<&Prices>,
) -> Result<Prices, Error> {
    let prices = match (custom_input, custom_output) {
        (Some(input), Some(output)) => Some(Prices {
            input_cost_per_token: input,
            output_cost_per_token: output,
            ..Prices::default()
        }),
        _ => catalog.copied(),
    };
    prices
        .filter(|rates| rates.input_cost_per_token >= 0.0 && rates.output_cost_per_token >= 0.0)
        .ok_or_else(|| Error::Pricing {
            model: model.into(),
        })
}

pub fn deployment_exceeds_context(
    prompt_tokens: u64,
    configured: Option<NonZeroU64>,
    catalog: Option<NonZeroU64>,
) -> bool {
    configured
        .or(catalog)
        .is_some_and(|capacity| prompt_tokens >= capacity.get())
}

pub fn exceeds_context(deployments: &[(u64, ModelCapacity, ModelCapacity)]) -> bool {
    deployments.iter().all(|(tokens, configured, catalog)| {
        deployment_exceeds_context(
            *tokens,
            configured.max_input_tokens,
            catalog.max_input_tokens,
        )
    })
}

pub fn output_tokens(
    model: &str,
    limits: &OutputLimits,
    catalog: ModelCapacity,
    prompt_tokens: Option<u64>,
) -> Result<u64, Error> {
    let capacity = limits
        .max_completion_tokens
        .or(limits.max_tokens)
        .or(limits.max_output_tokens)
        .or(catalog.max_output_tokens)
        .ok_or_else(|| Error::OutputCapacity {
            model: model.into(),
        })?
        .get();
    let (Some(prompt), Some(max_output)) = (prompt_tokens, catalog.max_output_tokens) else {
        return Ok(capacity);
    };
    let maximum = max_output.get();
    if catalog.max_input_tokens == Some(max_output) {
        return Ok(if prompt <= maximum {
            capacity.min(maximum - prompt)
        } else {
            capacity
        });
    }
    Ok(capacity.min(maximum))
}

pub fn quote(deployments: &[DeploymentEstimate], cacheable: bool) -> Result<f64, Error> {
    let first = deployments.first().ok_or(Error::ModelUnavailable)?;
    let input_rate = deployments
        .iter()
        .map(|deployment| {
            let p = deployment.prices;
            let base = p
                .input_cost_per_token
                .max(p.input_cost_per_token_above_200k_tokens)
                .max(p.input_cost_per_token_above_128k_tokens)
                .max(p.input_cost_per_token_above_272k_tokens);
            if cacheable {
                base.max(p.cache_creation_input_token_cost)
                    .max(p.cache_creation_input_token_cost_above_200k_tokens)
                    .max(p.cache_creation_input_token_cost_above_272k_tokens)
            } else {
                base
            }
        })
        .fold(first.prices.input_cost_per_token, f64::max);
    let output_rate = deployments
        .iter()
        .map(|deployment| {
            let p = deployment.prices;
            p.output_cost_per_token
                .max(p.output_cost_per_token_above_200k_tokens)
                .max(p.output_cost_per_token_above_128k_tokens)
                .max(p.output_cost_per_token_above_272k_tokens)
        })
        .fold(first.prices.output_cost_per_token, f64::max);
    let input = deployments
        .iter()
        .map(|deployment| deployment.prompt_tokens)
        .fold(first.prompt_tokens, u64::max);
    let output = deployments
        .iter()
        .map(|deployment| deployment.output_tokens)
        .fold(first.output_tokens, u64::min);
    Ok(input as f64 * input_rate + output as f64 * output_rate)
}
