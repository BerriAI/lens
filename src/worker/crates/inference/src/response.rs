use crate::{Error, Prices};
use chrono::{DateTime, Utc};
use lens_contract::worker::{
    ModelRequest, ModelRequestPurpose, ModelResult, ModelResultFinishReason, Step, StepKind,
};
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct Usage {
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct UsageEnvelope {
    pub model: Option<String>,
    pub usage: Option<Usage>,
}

pub fn model_step(
    envelope: &UsageEnvelope,
    body: &ModelRequest,
    requested: &str,
    cost: f64,
    now: DateTime<Utc>,
) -> Result<Step, Error> {
    let label = match body.purpose {
        ModelRequestPurpose::Extract => "Reviewed a run",
        ModelRequestPurpose::Cluster => "Compared observations",
        ModelRequestPurpose::Investigate => "Checked a pattern",
    };
    let usage = envelope.usage.unwrap_or_default();
    Ok(Step {
        at: now,
        kind: StepKind::Model,
        label: label.try_into()?,
        model: envelope
            .model
            .as_deref()
            .filter(|model| !model.is_empty())
            .unwrap_or(requested)
            .try_into()?,
        purpose: body.purpose.to_string().try_into()?,
        prompt_tokens: usage.prompt_tokens.unwrap_or(0),
        completion_tokens: usage.completion_tokens.unwrap_or(0),
        cost,
    })
}

pub fn completion_charge(custom: Option<&Prices>, usage: Usage, actual: f64, estimate: f64) -> f64 {
    if let Some(prices) = custom {
        return usage.prompt_tokens.unwrap_or(0) as f64 * prices.input_cost_per_token
            + usage.completion_tokens.unwrap_or(0) as f64 * prices.output_cost_per_token;
    }
    if actual > 0.0 { actual } else { estimate }
}

pub fn model_result(content: Option<&str>, cost: f64, finish_reason: Option<&str>) -> ModelResult {
    ModelResult {
        content: content.unwrap_or_default().into(),
        cost,
        context_exceeded: false,
        finish_reason: match finish_reason {
            Some("length") => Some(ModelResultFinishReason::Length),
            Some("content_filter") => Some(ModelResultFinishReason::ContentFilter),
            _ => None,
        },
    }
}
