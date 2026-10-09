use std::{collections::BTreeSet, num::NonZeroU64};

use lens_inference::{ModelCapacity, OutputLimits};
use serde::Deserialize;

use super::GatewayConfig;
use crate::error::GatewayError;

const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const ANALYSIS_OUTPUT_TOKENS: u64 = 4096;

#[derive(Deserialize)]
struct Catalog {
    data: Vec<Model>,
}

#[derive(Deserialize)]
struct Model {
    model_group: String,
    mode: Option<String>,
    input_cost_per_token: Option<f64>,
    output_cost_per_token: Option<f64>,
    max_input_tokens: Option<f64>,
    max_output_tokens: Option<f64>,
}

pub(super) struct Discovered {
    pub analysis: Vec<lens_analysis::Deployment>,
    pub evaluation: Vec<lens_decisions::Deployment>,
    pub unavailable: usize,
}

pub(super) async fn fetch(
    client: &reqwest::Client,
    connection: &GatewayConfig,
    analysis: &[lens_analysis::Deployment],
    evaluation: &[lens_decisions::Deployment],
) -> Result<Discovered, GatewayError> {
    let endpoint = format!(
        "{}/model_group/info",
        connection.base.as_str().trim_end_matches('/')
    );
    let mut response = client
        .get(endpoint)
        .bearer_auth(&connection.key)
        .send()
        .await
        .map_err(|_| GatewayError::Connection)?;
    if !response.status().is_success() {
        return Err(GatewayError::Rejected {
            status: response.status().as_u16(),
        });
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(GatewayError::ResponseLimit);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| GatewayError::Connection)?
    {
        if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(GatewayError::ResponseLimit);
        }
        bytes.extend_from_slice(&chunk);
    }
    let catalog: Catalog = serde_json::from_slice(&bytes).map_err(|_| GatewayError::Response)?;
    let bundled = lens_analysis::bundled_catalog().map_err(|_| GatewayError::Models)?;
    let mut aliases: BTreeSet<_> = analysis
        .iter()
        .map(|model| model.name.clone())
        .chain(evaluation.iter().map(|model| model.name.clone()))
        .collect();
    let mut discovered = Discovered {
        analysis: Vec::new(),
        evaluation: Vec::new(),
        unavailable: 0,
    };
    for model in catalog.data {
        if model.model_group.trim().is_empty() || !aliases.insert(model.model_group.clone()) {
            continue;
        }
        match model.mode.as_deref() {
            Some("evaluation") => {
                let deployment = lens_decisions::Deployment {
                    name: model.model_group.clone(),
                    model: model.model_group,
                    provider: lens_decisions::Provider::DecisionsCompatible,
                    api_base: Some(connection.base.clone()),
                    api_key: Some(lens_decisions::Secret::new(connection.key.clone())),
                };
                if lens_decisions::EvaluationModels::new(
                    bundled.clone(),
                    vec![deployment.clone()],
                    Default::default(),
                )
                .is_ok()
                {
                    discovered.evaluation.push(deployment);
                } else {
                    discovered.unavailable += 1;
                }
            }
            Some("chat") => {
                let Some(deployment) = analysis_deployment(model, connection) else {
                    discovered.unavailable += 1;
                    continue;
                };
                if lens_analysis::AnalysisModels::new(
                    bundled.clone(),
                    vec![deployment.clone()],
                    Default::default(),
                )
                .is_ok()
                {
                    discovered.analysis.push(deployment);
                } else {
                    discovered.unavailable += 1;
                }
            }
            _ => {}
        }
    }
    Ok(discovered)
}

fn analysis_deployment(
    model: Model,
    connection: &GatewayConfig,
) -> Option<lens_analysis::Deployment> {
    let input = model
        .input_cost_per_token
        .filter(|cost| cost.is_finite() && *cost >= 0.0)?;
    let output = model
        .output_cost_per_token
        .filter(|cost| cost.is_finite() && *cost >= 0.0)?;
    let output_capacity = capacity(model.max_output_tokens?)?;
    let input_capacity = match model.max_input_tokens {
        Some(value) => Some(capacity(value)?),
        None => None,
    };
    let mut base = connection.base.clone();
    base.set_path(&format!("{}/v1", base.path().trim_end_matches('/')));
    Some(lens_analysis::Deployment {
        name: model.model_group.clone(),
        model: model.model_group,
        provider: lens_analysis::Provider::OpenAiCompatible,
        api_base: Some(base),
        api_key: lens_analysis::Secret::new(connection.key.clone()),
        input_cost_per_token: Some(input),
        output_cost_per_token: Some(output),
        capacity: ModelCapacity {
            max_input_tokens: input_capacity,
            max_output_tokens: Some(output_capacity),
        },
        output_limits: OutputLimits {
            max_output_tokens: NonZeroU64::new(output_capacity.get().min(ANALYSIS_OUTPUT_TOKENS)),
            ..Default::default()
        },
    })
}

fn capacity(value: f64) -> Option<NonZeroU64> {
    if !value.is_finite() || value.fract() != 0.0 || value < 1.0 || value >= u64::MAX as f64 {
        return None;
    }
    NonZeroU64::new(value as u64)
}

#[cfg(test)]
mod tests {
    use super::{GatewayConfig, Model, analysis_deployment};
    use rstest::rstest;

    #[rstest]
    #[case::large_context(128_000, 4096)]
    #[case::small_context(512, 512)]
    fn analysis_limits_requested_output_without_losing_model_capacity(
        #[case] capacity: u64,
        #[case] requested: u64,
    ) {
        let model = Model {
            model_group: "analysis".into(),
            mode: Some("chat".into()),
            input_cost_per_token: Some(0.0),
            output_cost_per_token: Some(0.0),
            max_input_tokens: Some(200_000.0),
            max_output_tokens: Some(capacity as f64),
        };
        let deployment = analysis_deployment(
            model,
            &GatewayConfig::new("https://gateway.example", "fixture-key".into()).unwrap(),
        )
        .unwrap();
        assert_eq!(
            deployment.capacity.max_output_tokens.unwrap().get(),
            capacity
        );
        assert_eq!(deployment.capacity.max_input_tokens.unwrap().get(), 200_000);
        assert_eq!(
            deployment.output_limits.max_output_tokens.unwrap().get(),
            requested
        );
    }
}
