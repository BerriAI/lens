use super::{Completion, Outcome, StrictSchema};
use crate::{
    Error,
    catalog::{CacheMarks, ResolvedDeployment},
};
use lens_contract::worker::{ModelMessage, ModelMessageRole};
use litellm_core_utils::{
    core_helpers::finish_reason_for, prompt_templates::factory::build_conversation,
};
use litellm_cost::{PromptConvention, Usage};
use litellm_llms_types::formats::chat_completions::ChatMessage;
use serde::Deserialize;
use serde_json::{Value, json};

fn text_block(text: &str, cached: bool) -> Value {
    if cached {
        json!({"type":"text","text":text,"cache_control":{"type":"ephemeral"}})
    } else {
        json!({"type":"text","text":text})
    }
}

pub(super) fn request(
    deployment: &ResolvedDeployment,
    messages: &[ModelMessage],
    boundaries: &[usize],
    output: u64,
    schema: Option<&StrictSchema>,
    stream: bool,
) -> Result<Value, Error> {
    let source: Vec<ChatMessage> = serde_json::from_value(json!(messages))?;
    let conversation = build_conversation(&source);
    if !conversation.opens_on_user_turn() {
        return Err(Error::Conversation);
    }
    let cached =
        |index: usize| deployment.cache_marks != CacheMarks::None && boundaries.contains(&index);
    let system_marks = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| {
            message.role == ModelMessageRole::System && !message.content.is_empty()
        })
        .map(|(index, _)| cached(index));
    let system: Vec<_> = conversation
        .system
        .iter()
        .zip(system_marks)
        .map(|(text, cache)| text_block(text, cache))
        .collect();
    let mut turn_marks = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message.role != ModelMessageRole::System)
        .map(|(index, _)| cached(index));
    let turns: Vec<_> = conversation
        .turns
        .iter()
        .map(|turn| {
            let blocks: Vec<_> = turn
                .texts
                .iter()
                .map(|text| text_block(text, turn_marks.next().unwrap_or(false)))
                .collect();
            let role: &'static str = turn.role.into();
            json!({"role":role,"content":blocks})
        })
        .collect();
    let mut body = json!({"model":deployment.model,"system":system,"messages":turns,"max_tokens":output,"stream":stream});
    if let Some(schema) = schema {
        body["output_config"] = json!({"format":{"type":"json_schema","schema":schema.schema}});
    }
    Ok(body)
}

#[derive(Deserialize)]
struct Response {
    model: String,
    content: Vec<Block>,
    stop_reason: Option<String>,
    stop_details: Option<StopDetails>,
    usage: ResponseUsage,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Block {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
    },
    RedactedThinking {
        data: String,
    },
    #[serde(other)]
    Unsupported,
}
#[derive(Deserialize)]
struct ResponseUsage {
    input_tokens: u64,
    output_tokens: u64,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_creation: Option<CacheCreation>,
}
#[derive(Deserialize)]
struct CacheCreation {
    ephemeral_5m_input_tokens: u64,
    ephemeral_1h_input_tokens: u64,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Event {
    MessageStart {
        message: Opening,
    },
    ContentBlockStart {
        content_block: Block,
    },
    ContentBlockDelta {
        delta: Delta,
    },
    MessageDelta {
        delta: Closing,
        usage: Option<FinalUsage>,
    },
    Error {},
    #[serde(other)]
    Other,
}
#[derive(Deserialize)]
struct Opening {
    model: String,
    usage: ResponseUsage,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Delta {
    TextDelta {
        text: String,
    },
    #[serde(other)]
    Other,
}
#[derive(Deserialize)]
struct Closing {
    stop_reason: Option<String>,
    stop_details: Option<StopDetails>,
}
#[derive(Deserialize)]
struct StopDetails {
    explanation: Option<String>,
}
#[derive(Deserialize)]
struct FinalUsage {
    output_tokens: u64,
}

fn text(block: Block) -> Result<String, Error> {
    match block {
        Block::Text { text } => Ok(text),
        Block::Thinking { thinking } => {
            drop(thinking);
            Ok(String::new())
        }
        Block::RedactedThinking { data } => {
            drop(data);
            Ok(String::new())
        }
        Block::Unsupported => Err(Error::ResponseContent),
    }
}

pub(super) fn response(body: &[u8]) -> Result<Completion, Error> {
    let response: Response = serde_json::from_slice(body)?;
    let content = response
        .content
        .into_iter()
        .map(text)
        .collect::<Result<String, _>>()?;
    Ok(completion(
        response.model,
        content,
        response.stop_reason,
        response.stop_details,
        response.usage,
    ))
}

pub(super) fn stream(body: &[u8]) -> Result<Outcome<'_>, Error> {
    let mut opening = None;
    let mut content = String::new();
    let mut closing = None;
    let mut output_tokens = None;
    let mut events = super::events(body).peekable();
    while let Some(event) = events.next() {
        let Some(parsed) = super::event::<Event>(event, events.peek().is_none())? else {
            break;
        };
        match parsed {
            Event::MessageStart { message } => opening = Some(message),
            Event::ContentBlockStart { content_block } => content.push_str(&text(content_block)?),
            Event::ContentBlockDelta {
                delta: Delta::TextDelta { text },
            } => content.push_str(&text),
            Event::MessageDelta { delta, usage } => {
                closing = Some(delta);
                output_tokens = usage.map(|usage| usage.output_tokens).or(output_tokens);
            }
            Event::Error {} => return Ok(Outcome::Failed(event)),
            Event::ContentBlockDelta { .. } | Event::Other => {}
        }
    }
    let (Some(opening), Some(closing)) = (opening, closing) else {
        return Ok(Outcome::Cut);
    };
    let usage = ResponseUsage {
        output_tokens: output_tokens.unwrap_or(opening.usage.output_tokens),
        ..opening.usage
    };
    Ok(Outcome::Complete(completion(
        opening.model,
        content,
        closing.stop_reason,
        closing.stop_details,
        usage,
    )))
}

fn completion(
    model: String,
    content: String,
    stop_reason: Option<String>,
    stop_details: Option<StopDetails>,
    usage: ResponseUsage,
) -> Completion {
    Completion {
        model,
        content: (!content.is_empty()).then_some(content),
        refusal: stop_details
            .filter(|_| stop_reason.as_deref() == Some("refusal"))
            .and_then(|details| details.explanation)
            .filter(|reason| !reason.trim().is_empty()),
        finish_reason: stop_reason.map(|reason| finish_reason_for(&reason).to_owned()),
        usage: Usage {
            prompt_tokens: usage.input_tokens,
            completion_tokens: usage.output_tokens,
            cache_read_tokens: usage.cache_read_input_tokens.unwrap_or(0),
            cache_write_tokens: usage.cache_creation_input_tokens.unwrap_or(0),
            cache_write_5m_tokens: usage
                .cache_creation
                .as_ref()
                .map(|details| details.ephemeral_5m_input_tokens),
            cache_write_1h_tokens: usage
                .cache_creation
                .as_ref()
                .map(|details| details.ephemeral_1h_input_tokens),
            prompt_convention: PromptConvention::ExcludesCache,
        },
    }
}
