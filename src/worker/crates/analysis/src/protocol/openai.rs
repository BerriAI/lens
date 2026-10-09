use super::Completion;
use crate::{Error, Provider, catalog::ResolvedDeployment};
use lens_contract::worker::ModelMessage;
use litellm_cost::{PromptConvention, Usage};
use serde::Deserialize;
use serde_json::{Value, json};

pub(super) fn request(
    deployment: &ResolvedDeployment,
    messages: &[ModelMessage],
    boundaries: &[usize],
    output: u64,
) -> Result<Value, Error> {
    let messages: Vec<_> = messages.iter().enumerate().map(|(index,message)| {
        let content = if deployment.cache_breakpoints && boundaries.contains(&index) {
            json!([{"type":"text","text":message.content,"prompt_cache_breakpoint":{"mode":"explicit"}}])
        } else { json!(message.content) };
        json!({"role":message.role,"content":content})
    }).collect();
    let limit = if deployment.config.provider == Provider::OpenAi {
        "max_completion_tokens"
    } else {
        "max_tokens"
    };
    Ok(
        json!({"model":deployment.model,"messages":messages,limit:output,"stream":false,"response_format":{"type":"json_object"}}),
    )
}

#[derive(Deserialize)]
struct Response {
    model: Option<String>,
    choices: Vec<Choice>,
    usage: Option<ResponseUsage>,
}

#[derive(Deserialize)]
struct Choice {
    message: Message,
    finish_reason: Option<String>,
}
#[derive(Deserialize)]
struct Message {
    content: Option<String>,
}
#[derive(Default, Deserialize)]
struct ResponseUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    prompt_tokens_details: Option<PromptDetails>,
}
#[derive(Default, Deserialize)]
struct PromptDetails {
    cached_tokens: Option<u64>,
    cache_creation_tokens: Option<u64>,
}

pub(super) fn response(model: &str, body: &[u8]) -> Result<Completion, Error> {
    let response: Response = serde_json::from_slice(body)?;
    let choice = response
        .choices
        .into_iter()
        .next()
        .ok_or(Error::MissingCompletion)?;
    let usage = response.usage.unwrap_or_default();
    let details = usage.prompt_tokens_details.unwrap_or_default();
    Ok(Completion {
        model: response
            .model
            .filter(|model| !model.is_empty())
            .unwrap_or_else(|| model.into()),
        content: choice.message.content,
        finish_reason: choice.finish_reason,
        usage: Usage {
            prompt_tokens: usage.prompt_tokens.unwrap_or(0),
            completion_tokens: usage.completion_tokens.unwrap_or(0),
            cache_read_tokens: details.cached_tokens.unwrap_or(0),
            cache_write_tokens: details.cache_creation_tokens.unwrap_or(0),
            cache_write_5m_tokens: None,
            cache_write_1h_tokens: None,
            prompt_convention: PromptConvention::IncludesCache,
        },
    })
}
