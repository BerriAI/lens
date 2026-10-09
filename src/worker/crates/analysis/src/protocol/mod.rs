mod anthropic;
mod openai;

use crate::{Error, Provider, catalog::ResolvedDeployment};
use lens_contract::worker::ModelMessage;
use serde::Deserialize;
use serde_json::Value;

pub(crate) struct Completion {
    pub(crate) model: String,
    pub(crate) content: Option<String>,
    pub(crate) finish_reason: Option<String>,
    pub(crate) usage: litellm_cost::Usage,
}

pub(crate) fn request(
    deployment: &ResolvedDeployment,
    messages: &[ModelMessage],
    boundaries: &[usize],
    output: u64,
) -> Result<Value, Error> {
    match deployment.config.provider {
        Provider::Anthropic => anthropic::request(deployment, messages, boundaries, output),
        Provider::OpenAi | Provider::OpenAiCompatible => {
            openai::request(deployment, messages, boundaries, output)
        }
    }
}

pub(crate) fn response(provider: Provider, model: &str, body: &[u8]) -> Result<Completion, Error> {
    match provider {
        Provider::Anthropic => anthropic::response(body),
        Provider::OpenAi | Provider::OpenAiCompatible => openai::response(model, body),
    }
}

#[derive(Deserialize)]
struct Failure {
    error: Option<FailureDetail>,
}
#[derive(Deserialize)]
struct FailureDetail {
    code: Option<Value>,
    #[serde(rename = "type")]
    kind: Option<String>,
    message: Option<String>,
}

pub(crate) fn context_failure(provider: Provider, body: &[u8]) -> bool {
    let Ok(Failure { error: Some(error) }) = serde_json::from_slice(body) else {
        return false;
    };
    error.code.as_ref().and_then(Value::as_str) == Some("context_length_exceeded")
        || error.kind.as_deref() == Some("context_window_exceeded")
        || (provider == Provider::Anthropic
            && error.kind.as_deref() == Some("invalid_request_error")
            && error.message.as_deref().is_some_and(|message| {
                message.starts_with("prompt is too long:")
                    || message.contains("exceeds the context window")
            }))
}
