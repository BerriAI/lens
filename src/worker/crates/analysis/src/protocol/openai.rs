use super::{Completion, Outcome, StrictSchema};
use crate::{
    Error, Provider,
    catalog::{CacheMarks, ResolvedDeployment},
};
use lens_contract::worker::{ModelMessage, ModelMessageRole};
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
    let mark = match deployment.cache_marks {
        CacheMarks::None => None,
        CacheMarks::Native => Some(("prompt_cache_breakpoint", json!({"mode":"explicit"}))),
        CacheMarks::Forwarded => Some(("cache_control", json!({"type":"ephemeral"}))),
    };
    let parts: Vec<_> = messages
        .iter()
        .enumerate()
        .map(|(index, message)| Part {
            role: message.role,
            text: message.content.clone(),
            marked: boundaries.contains(&index),
        })
        .collect();
    let parts = if deployment.config.provider == Provider::OpenAiCompatible {
        single_leading_system(parts)
    } else {
        parts
    };
    let messages: Vec<_> = parts
        .into_iter()
        .map(|part| {
            let content = match &mark {
                Some((name, value)) if part.marked => {
                    json!([{"type":"text","text":part.text,*name:value}])
                }
                _ => json!(part.text),
            };
            json!({"role":part.role,"content":content})
        })
        .collect();
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

struct Part {
    role: ModelMessageRole,
    text: String,
    marked: bool,
}

fn single_leading_system(parts: Vec<Part>) -> Vec<Part> {
    let leading = parts
        .iter()
        .take_while(|part| part.role == ModelMessageRole::System)
        .count();
    let mut parts = parts.into_iter();
    let opening: Vec<_> = parts.by_ref().take(leading).collect();
    let system = (!opening.is_empty()).then(|| Part {
        role: ModelMessageRole::System,
        marked: opening.iter().any(|part| part.marked),
        text: opening
            .into_iter()
            .map(|part| part.text)
            .collect::<Vec<_>>()
            .join("\n\n"),
    });
    system
        .into_iter()
        .chain(parts.map(|part| Part {
            role: if part.role == ModelMessageRole::System {
                ModelMessageRole::User
            } else {
                part.role
            },
            ..part
        }))
        .collect()
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
    refusal: Option<String>,
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
        choice.message.refusal,
        choice.finish_reason,
        response.usage,
    ))
}

pub(super) fn stream<'a>(model: &str, body: &'a [u8]) -> Result<Outcome<'a>, Error> {
    let mut reported = None;
    let mut content = String::new();
    let mut refusal = String::new();
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
            if let Some(delta) = choice.delta {
                content.push_str(delta.content.as_deref().unwrap_or_default());
                refusal.push_str(delta.refusal.as_deref().unwrap_or_default());
            }
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
        (!refusal.is_empty()).then_some(refusal),
        finish_reason,
        usage,
    )))
}

fn completion(
    model: &str,
    reported: Option<String>,
    content: Option<String>,
    refusal: Option<String>,
    finish_reason: Option<String>,
    usage: Option<ResponseUsage>,
) -> Completion {
    let usage = usage.unwrap_or_default();
    let details = usage.prompt_tokens_details.unwrap_or_default();
    let refusal = refusal.filter(|reason| !reason.trim().is_empty());
    Completion {
        model: reported
            .filter(|model| !model.is_empty())
            .unwrap_or_else(|| model.into()),
        content,
        finish_reason: if refusal.is_some() {
            Some("content_filter".into())
        } else {
            finish_reason
        },
        refusal,
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
