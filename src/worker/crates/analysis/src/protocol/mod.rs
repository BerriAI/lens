mod anthropic;
mod openai;
mod schema;

use crate::{Error, Provider, ResponseSchema, catalog::ResolvedDeployment};
use lens_contract::worker::ModelMessage;
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Completion {
    pub(crate) model: String,
    pub(crate) content: Option<String>,
    pub(crate) finish_reason: Option<String>,
    pub(crate) refusal: Option<String>,
    pub(crate) usage: litellm_cost::Usage,
}

pub(crate) enum Outcome<'a> {
    Complete(Completion),
    Failed(&'a [u8]),
    Cut,
}

pub(crate) struct StrictSchema {
    name: String,
    schema: Value,
}

pub(crate) fn strict_schema(format: &ResponseSchema<'_>) -> Option<StrictSchema> {
    Some(StrictSchema {
        name: schema::name(format.name),
        schema: schema::strict(format.schema)?,
    })
}

pub(crate) fn request(
    deployment: &ResolvedDeployment,
    messages: &[ModelMessage],
    boundaries: &[usize],
    output: u64,
    schema: Option<&StrictSchema>,
    stream: bool,
) -> Result<Value, Error> {
    match deployment.config.provider {
        Provider::Anthropic => {
            anthropic::request(deployment, messages, boundaries, output, schema, stream)
        }
        Provider::OpenAi | Provider::OpenAiCompatible => {
            openai::request(deployment, messages, boundaries, output, schema, stream)
        }
    }
}

pub(crate) fn low_reasoning(provider: Provider, body: &Value) -> Option<Value> {
    (provider != Provider::Anthropic).then(|| {
        let mut body = body.clone();
        body["reasoning_effort"] = json!("low");
        body
    })
}

pub(crate) fn response<'a>(
    provider: Provider,
    model: &str,
    body: &'a [u8],
    streamed: bool,
) -> Result<Outcome<'a>, Error> {
    match (provider, streamed) {
        (Provider::Anthropic, true) => anthropic::stream(body),
        (Provider::Anthropic, false) => anthropic::response(body).map(Outcome::Complete),
        (Provider::OpenAi | Provider::OpenAiCompatible, true) => openai::stream(model, body),
        (Provider::OpenAi | Provider::OpenAiCompatible, false) => {
            openai::response(model, body).map(Outcome::Complete)
        }
    }
}

fn events(body: &[u8]) -> impl Iterator<Item = &[u8]> {
    body.split(|byte| *byte == b'\n')
        .filter_map(|line| line.strip_prefix(b"data:"))
        .map(<[u8]>::trim_ascii)
        .filter(|data| !data.is_empty() && *data != b"[DONE]")
}

fn event<'a, T: Deserialize<'a>>(
    data: &'a [u8],
    last: bool,
) -> Result<Option<T>, serde_json::Error> {
    match serde_json::from_slice(data) {
        Ok(event) => Ok(Some(event)),
        Err(_) if last => Ok(None),
        Err(error) => Err(error),
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
        || error
            .message
            .as_deref()
            .is_some_and(|message| message.starts_with("litellm.ContextWindowExceededError"))
}
