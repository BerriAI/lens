use super::Completion;
use crate::{Error, catalog::ResolvedDeployment};
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
) -> Result<Value, Error> {
    let source: Vec<ChatMessage> = serde_json::from_value(json!(messages))?;
    let conversation = build_conversation(&source);
    if !conversation.opens_on_user_turn() {
        return Err(Error::Conversation);
    }
    let cached = |index: usize| deployment.cache_breakpoints && boundaries.contains(&index);
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
    Ok(
        json!({"model":deployment.model,"system":system,"messages":turns,"max_tokens":output,"stream":false}),
    )
}

#[derive(Deserialize)]
struct Response {
    model: String,
    content: Vec<Block>,
    stop_reason: Option<String>,
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

pub(super) fn response(body: &[u8]) -> Result<Completion, Error> {
    let response: Response = serde_json::from_slice(body)?;
    let content = response
        .content
        .into_iter()
        .map(|block| match block {
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
        })
        .collect::<Result<String, _>>()?;
    Ok(Completion {
        model: response.model,
        content: (!content.is_empty()).then_some(content),
        finish_reason: response
            .stop_reason
            .map(|reason| finish_reason_for(&reason).to_owned()),
        usage: Usage {
            prompt_tokens: response.usage.input_tokens,
            completion_tokens: response.usage.output_tokens,
            cache_read_tokens: response.usage.cache_read_input_tokens.unwrap_or(0),
            cache_write_tokens: response.usage.cache_creation_input_tokens.unwrap_or(0),
            cache_write_5m_tokens: response
                .usage
                .cache_creation
                .as_ref()
                .map(|details| details.ephemeral_5m_input_tokens),
            cache_write_1h_tokens: response
                .usage
                .cache_creation
                .as_ref()
                .map(|details| details.ephemeral_1h_input_tokens),
            prompt_convention: PromptConvention::ExcludesCache,
        },
    })
}
