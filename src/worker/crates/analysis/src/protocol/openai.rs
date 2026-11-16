use super::{Completion, Outcome, StrictSchema};
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
    schema: Option<&StrictSchema>,
    stream: bool,
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
    let format = match schema {
        Some(schema) => {
            json!({"type":"json_schema","json_schema":{"name":schema.name,"strict":true,"schema":schema.schema}})
        }
        None => json!({"type":"json_object"}),
    };
    let mut body = json!({"model":deployment.model,"messages":messages,limit:output,"stream":stream,"response_format":format});
    if stream {
        body["stream_options"] = json!({"include_usage":true});
    }
    Ok(body)
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

#[derive(Deserialize)]
struct Chunk {
    model: Option<String>,
    #[serde(default)]
    choices: Vec<ChunkChoice>,
    usage: Option<ResponseUsage>,
    error: Option<serde::de::IgnoredAny>,
}

#[derive(Deserialize)]
struct ChunkChoice {
    delta: Option<Message>,
    finish_reason: Option<String>,
}

pub(super) fn response(model: &str, body: &[u8]) -> Result<Completion, Error> {
    let response: Response = serde_json::from_slice(body)?;
    let choice = response
        .choices
        .into_iter()
        .next()
        .ok_or(Error::MissingCompletion)?;
    Ok(completion(
        model,
        response.model,
        choice.message.content,
        choice.finish_reason,
        response.usage,
    ))
}

pub(super) fn stream<'a>(model: &str, body: &'a [u8]) -> Result<Outcome<'a>, Error> {
    let mut reported = None;
    let mut content = String::new();
    let mut finish_reason = None;
    let mut usage = None;
    let mut events = super::events(body).peekable();
    while let Some(event) = events.next() {
        let Some(chunk) = super::event::<Chunk>(event, events.peek().is_none())? else {
            break;
        };
        if chunk.error.is_some() {
            return Ok(Outcome::Failed(event));
        }
        reported = reported.or(chunk.model);
        usage = chunk.usage.or(usage);
        if let Some(choice) = chunk.choices.into_iter().next() {
            content.push_str(
                choice
                    .delta
                    .and_then(|delta| delta.content)
                    .as_deref()
                    .unwrap_or_default(),
            );
            finish_reason = choice.finish_reason.or(finish_reason);
        }
    }
    if finish_reason.is_none() {
        return Ok(Outcome::Cut);
    }
    Ok(Outcome::Complete(completion(
        model,
        reported,
        (!content.is_empty()).then_some(content),
        finish_reason,
        usage,
    )))
}

fn completion(
    model: &str,
    reported: Option<String>,
    content: Option<String>,
    finish_reason: Option<String>,
    usage: Option<ResponseUsage>,
) -> Completion {
    let usage = usage.unwrap_or_default();
    let details = usage.prompt_tokens_details.unwrap_or_default();
    Completion {
        model: reported
            .filter(|model| !model.is_empty())
            .unwrap_or_else(|| model.into()),
        content,
        finish_reason,
        usage: Usage {
            prompt_tokens: usage.prompt_tokens.unwrap_or(0),
            completion_tokens: usage.completion_tokens.unwrap_or(0),
            cache_read_tokens: details.cached_tokens.unwrap_or(0),
            cache_write_tokens: details.cache_creation_tokens.unwrap_or(0),
            cache_write_5m_tokens: None,
            cache_write_1h_tokens: None,
            prompt_convention: PromptConvention::IncludesCache,
        },
    }
}
