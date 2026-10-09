use crate::{Deployment, Error, Provider, tokens};
use lens_inference::{ModelCapacity, Prices, deployment_prices, output_tokens};
use litellm_cost::{Pricing, PricingPlan, Rate, Rates, ThresholdRates};
use litellm_model_catalog::{Catalog, Mode, ModelEntry, Provenance};
use litellm_token_counter::TokenCounter;
use std::{num::NonZeroU64, sync::Arc};
use url::Url;

pub fn bundled_catalog() -> Result<Arc<Catalog>, Error> {
    let bytes = normalize_snapshot(include_bytes!("../data/model_catalog.json"))?;
    Ok(Arc::new(Catalog::parse(&bytes, Provenance {
        source: Some("https://github.com/BerriAI/litellm/blob/0721cffab2ecbde51cdef9aeca0ce1c16b3e0aa9/model_prices_and_context_window.json".into()),
        revision: Some("0721cffab2ecbde51cdef9aeca0ce1c16b3e0aa9".into()),
        etag: None,
    })?))
}

fn normalize_snapshot(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let mut snapshot: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(bytes)?;
    for fields in snapshot
        .values_mut()
        .filter_map(serde_json::Value::as_object_mut)
    {
        for key in [
            "max_input_tokens",
            "max_output_tokens",
            "max_tokens",
            "output_vector_size",
            "prompt_cache_min_tokens",
            "rpm",
            "tpm",
        ] {
            let Some(value) = fields.get_mut(key) else {
                continue;
            };
            if value.as_u64().is_none()
                && let Some(number) = value.as_f64()
                && number >= 0.0
                && number < u64::MAX as f64
                && number.fract() == 0.0
            {
                *value = (number as u64).into();
            }
        }
    }
    Ok(serde_json::to_vec(&snapshot)?)
}

pub(crate) struct ResolvedDeployment {
    pub(crate) config: Deployment,
    pub(crate) model: String,
    pub(crate) endpoint: Url,
    pub(crate) prices: Prices,
    pub(crate) catalog_capacity: ModelCapacity,
    pub(crate) pricing: PricingPlan,
    pub(crate) counter: Arc<TokenCounter>,
    pub(crate) cache_breakpoints: bool,
    pub(crate) legacy_accounting: bool,
}

pub(crate) fn resolve(config: Deployment, catalog: &Catalog) -> Result<ResolvedDeployment, Error> {
    if config.name.trim().is_empty() || config.model.trim().is_empty() {
        return Err(Error::MissingModel);
    }
    if config.api_key.expose().trim().is_empty() {
        return Err(Error::MissingCredential { model: config.name });
    }
    let direct = catalog.lookup(&config.model);
    let matched = direct.or_else(|| {
        config
            .model
            .split_once('/')
            .and_then(|(_, model)| catalog.lookup(model))
    });
    let entry = matched.map(|matched| matched.entry);
    if entry.is_some_and(|entry| entry.info().mode.is_some_and(|mode| mode != Mode::Chat)) {
        return Err(Error::ModelSurface {
            model: config.model,
        });
    }
    let catalog_prices = entry.and_then(|entry| {
        serde_json::from_value::<Prices>(serde_json::Value::Object(entry.fields().clone())).ok()
    });
    let prices = deployment_prices(
        &config.model,
        config.input_cost_per_token,
        config.output_cost_per_token,
        catalog_prices.as_ref(),
    )?;
    let catalog_capacity = ModelCapacity {
        max_input_tokens: entry
            .and_then(|entry| entry.info().max_input_tokens)
            .and_then(NonZeroU64::new),
        max_output_tokens: entry
            .and_then(|entry| entry.info().max_output_tokens)
            .and_then(NonZeroU64::new),
    };
    let limits = lens_inference::OutputLimits {
        max_output_tokens: config
            .output_limits
            .max_output_tokens
            .or(config.capacity.max_output_tokens),
        ..config.output_limits
    };
    output_tokens(&config.model, &limits, catalog_capacity, None)?;
    let config = Deployment {
        output_limits: limits,
        ..config
    };
    let model = match config.provider {
        Provider::OpenAi => config
            .model
            .strip_prefix("openai/")
            .unwrap_or(&config.model),
        Provider::Anthropic => config
            .model
            .strip_prefix("anthropic/")
            .unwrap_or(&config.model),
        Provider::OpenAiCompatible => &config.model,
    }
    .to_string();
    let endpoint = endpoint(config.provider, config.api_base.as_ref())?;
    let custom = config
        .input_cost_per_token
        .zip(config.output_cost_per_token);
    let pricing = pricing(entry, custom)?;
    let counter = Arc::new(tokens::counter(
        &config.model,
        direct.map(|matched| matched.entry),
    )?);
    let cache_breakpoints = entry.is_some_and(|entry| {
        if config.provider == Provider::Anthropic {
            entry.info().supports_prompt_caching == Some(true)
        } else {
            entry.info().supports_prompt_cache_breakpoint == Some(true)
        }
    });
    let legacy_accounting = config.model == "gpt-3.5-turbo-0301";
    Ok(ResolvedDeployment {
        config,
        model,
        endpoint,
        prices,
        catalog_capacity,
        pricing,
        counter,
        cache_breakpoints,
        legacy_accounting,
    })
}

fn endpoint(provider: Provider, configured: Option<&Url>) -> Result<Url, Error> {
    let mut endpoint = match configured {
        Some(endpoint) => endpoint.clone(),
        None => match provider {
            Provider::OpenAi => {
                Url::parse("https://api.openai.com/v1").map_err(|_| Error::Endpoint)?
            }
            Provider::Anthropic => {
                Url::parse("https://api.anthropic.com/v1").map_err(|_| Error::Endpoint)?
            }
            Provider::OpenAiCompatible => return Err(Error::Endpoint),
        },
    };
    if !matches!(endpoint.scheme(), "http" | "https")
        || endpoint.host_str().is_none()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(Error::Endpoint);
    }
    let suffix = if provider == Provider::Anthropic {
        "/messages"
    } else {
        "/chat/completions"
    };
    let path = endpoint.path().trim_end_matches('/');
    if !path.ends_with(suffix) {
        endpoint.set_path(&format!("{path}{suffix}"));
    }
    Ok(endpoint)
}

fn pricing(entry: Option<&ModelEntry>, custom: Option<(f64, f64)>) -> Result<PricingPlan, Error> {
    let base = match custom {
        Some((input, output)) => Rates {
            input: Rate::Value(input),
            output: Rate::Value(output),
            ..Rates::EMPTY
        },
        None => rates(entry, ""),
    };
    let thresholds: Vec<_> = if custom.is_some() {
        Vec::new()
    } else {
        [32_000, 100_000, 128_000, 200_000, 256_000, 272_000, 512_000]
            .into_iter()
            .map(|tokens| ThresholdRates {
                above_prompt_tokens: tokens,
                standard: rates(entry, &format!("_above_{}k_tokens", tokens / 1000)),
                tiers: &[],
            })
            .filter(|threshold| threshold.standard != Rates::EMPTY)
            .collect()
    };
    litellm_cost::compile(&Pricing {
        standard: base,
        tiers: &[],
        thresholds: &thresholds,
        off_peak: None,
    })
    .map_err(Error::Cost)
}

fn rates(entry: Option<&ModelEntry>, suffix: &str) -> Rates {
    let rate = |name: &str| {
        entry
            .and_then(|entry| entry.field(&format!("{name}{suffix}")))
            .and_then(serde_json::Value::as_f64)
            .map_or(Rate::Missing, Rate::Value)
    };
    Rates {
        input: rate("input_cost_per_token"),
        output: rate("output_cost_per_token"),
        cache_read: rate("cache_read_input_token_cost"),
        cache_write: rate("cache_creation_input_token_cost"),
        cache_write_1h: rate("cache_creation_input_token_cost_above_1hr"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use serde_json::{Value, json};

    #[rstest]
    #[case::integer(json!(1000),json!(1000))]
    #[case::integral_float(json!(1000.0),json!(1000))]
    #[case::zero_float(json!(0.0),json!(0))]
    #[case::fractional(json!(1000.25),json!(1000.25))]
    #[case::negative(json!(-1000.0),json!(-1000.0))]
    #[case::too_large(json!(18_446_744_073_709_551_616.0),json!(18_446_744_073_709_551_616.0))]
    #[case::null(json!(null),json!(null))]
    fn snapshot_normalizes_only_integral_capacity_fields(
        #[case] value: Value,
        #[case] expected: Value,
    ) {
        let bytes = serde_json::to_vec(
            &json!({"fixture":{"max_input_tokens":value,"input_cost_per_token":0.0}}),
        )
        .unwrap();
        let result: Value = serde_json::from_slice(&normalize_snapshot(&bytes).unwrap()).unwrap();
        assert_eq!(result["fixture"]["max_input_tokens"], expected);
        assert!(
            result["fixture"]["input_cost_per_token"]
                .as_number()
                .unwrap()
                .is_f64()
        );
    }

    #[rstest]
    #[case::native(Provider::OpenAi, Some("https://api.openai.com/v1/chat/completions"))]
    #[case::anthropic(Provider::Anthropic, Some("https://api.anthropic.com/v1/messages"))]
    #[case::compatible(Provider::OpenAiCompatible, None)]
    fn default_endpoint_requires_explicit_compatible_host(
        #[case] provider: Provider,
        #[case] expected: Option<&str>,
    ) {
        match expected {
            Some(expected) => assert_eq!(endpoint(provider, None).unwrap().as_str(), expected),
            None => assert!(matches!(endpoint(provider, None), Err(Error::Endpoint))),
        }
    }

    #[rstest]
    #[case::below(200_000, 2000.2)]
    #[case::above(200_001, 6000.53)]
    #[case::higher_empty_threshold_does_not_override(600_000, 18000.5)]
    fn cost_tiers_apply_with_base_fallbacks(#[case] input: u64, #[case] expected: f64) {
        let catalog=Catalog::parse(br#"{"fixture":{"input_cost_per_token":0.01,"output_cost_per_token":0.02,"input_cost_per_token_above_200k_tokens":0.03,"output_cost_per_token_above_200k_tokens":0.05}}"#,Default::default()).unwrap();
        let plan = pricing(Some(catalog.lookup("fixture").unwrap().entry), None).unwrap();
        let actual = plan
            .calculate(&litellm_cost::Request {
                usage: litellm_cost::Usage {
                    prompt_tokens: input,
                    completion_tokens: 10,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                    cache_write_5m_tokens: None,
                    cache_write_1h_tokens: None,
                    prompt_convention: litellm_cost::PromptConvention::IncludesCache,
                },
                service_tier: litellm_cost::ServiceTier::Standard,
                threshold_policy: litellm_cost::ThresholdPolicy::Exclusive,
                region_multiplier: None,
                billed_at_utc_minute: None,
            })
            .unwrap()
            .total();
        assert!((actual - expected).abs() < 1e-8);
    }
}
