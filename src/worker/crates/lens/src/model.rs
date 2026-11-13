use crate::{Error, control::JobClient, wire};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, VecDeque},
    sync::OnceLock,
};

pub fn schema(name: &str) -> Result<Value, Error> {
    static CONTRACT: OnceLock<Value> = OnceLock::new();
    let contract = CONTRACT.get_or_init(lens_contract::schema::worker_contract);
    let definitions = contract["definitions"]
        .as_object()
        .ok_or(Error::InvalidRequest)?;
    let mut root = definitions
        .get(name)
        .cloned()
        .ok_or(Error::InvalidRequest)?;
    let mut pending = VecDeque::new();
    references(&root, &mut pending);
    let mut selected = serde_json::Map::new();
    let mut seen = BTreeSet::new();
    while let Some(name) = pending.pop_front() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let definition = definitions.get(&name).ok_or(Error::InvalidRequest)?;
        references(definition, &mut pending);
        selected.insert(name, definition.clone());
    }
    root.as_object_mut()
        .ok_or(Error::InvalidRequest)?
        .insert("definitions".into(), selected.into());
    Ok(root)
}

fn references(value: &Value, found: &mut VecDeque<String>) {
    match value {
        Value::Object(object) => {
            if let Some(reference) = object
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|s| s.strip_prefix("#/definitions/"))
            {
                found.push_back(reference.into());
            }
            for value in object.values() {
                references(value, found);
            }
        }
        Value::Array(values) => {
            for value in values {
                references(value, found);
            }
        }
        _ => {}
    }
}

pub fn message(role: wire::ModelMessageRole, content: impl Into<String>) -> wire::ModelMessage {
    wire::ModelMessage {
        role,
        content: content.into(),
    }
}

pub fn request(
    purpose: wire::ModelRequestPurpose,
    prompt: Value,
) -> Result<wire::ModelRequest, Error> {
    Ok(wire::ModelRequest {
        purpose,
        messages: Vec::new(),
        prompt: serde_json::to_string(&prompt)?
            .try_into()
            .map_err(|_| Error::InvalidRequest)?,
    })
}

pub async fn structured<T: DeserializeOwned>(
    client: &JobClient,
    mut request: wire::ModelRequest,
    schema_name: &'static str,
    validate: impl Fn(&T) -> Option<String>,
) -> Result<(T, Vec<wire::ModelMessage>), Error> {
    let validator =
        jsonschema::validator_for(&schema(schema_name)?).map_err(|_| Error::InvalidRequest)?;
    let mut detail = String::new();
    for attempt in 0..2 {
        let response = client.model(&request).await?;
        if response.context_exceeded {
            return Err(Error::Context(Box::new(request)));
        }
        let value: Result<Value, _> = serde_json::from_str(&response.content);
        let contract_error = value
            .as_ref()
            .ok()
            .and_then(|value| validator.validate(value).err())
            .map(|error| error.to_string());
        let parsed: Result<T, _> = value.and_then(serde_json::from_value);
        detail = match parsed {
            Ok(ref value) if response.finish_reason.is_none() => contract_error
                .or_else(|| validate(value))
                .unwrap_or_default(),
            Ok(_) => "Model did not finish its response. Return a complete JSON object.".into(),
            Err(ref error) => error.to_string(),
        };
        if detail.is_empty() {
            request
                .messages
                .push(message(wire::ModelMessageRole::Assistant, response.content));
            return Ok((parsed?, request.messages));
        }
        if attempt == 0 {
            if request.messages.is_empty() {
                request.messages.push(message(
                    wire::ModelMessageRole::User,
                    request.prompt.to_string(),
                ));
            }
            request
                .messages
                .push(message(wire::ModelMessageRole::Assistant, response.content));
            request.messages.push(message(wire::ModelMessageRole::User, json!({
                "instruction": "Your previous response did not match the required response contract. Generate a new response from the original evidence, correcting the validation errors. Follow the complete object structure in response_schema. If the schema allows tools, you may request them before finalizing.",
                "validation_errors": detail,
                "response_schema": schema(schema_name)?,
            }).to_string()));
        }
    }
    Err(Error::ModelValidation {
        schema: schema_name,
        detail,
    })
}

fn history_resume(messages: &[wire::ModelMessage]) -> Result<wire::EvidenceRequest, Error> {
    let positions: Vec<Value> = messages
        .iter()
        .filter(|m| m.role == wire::ModelMessageRole::User)
        .filter_map(|m| serde_json::from_str(&m.content).ok())
        .collect();
    let visible = positions
        .iter()
        .filter_map(|p| p["journal_turns"].as_u64())
        .max()
        .unwrap_or_default();
    let checkpoint = positions
        .iter()
        .rposition(|p| p["resume_history_from_turn"].is_u64());
    let Some(index) = checkpoint else {
        return Ok(serde_json::from_value(
            json!({"action":"history", "turn_start":visible, "include_initial":true}),
        )?);
    };
    let position = &positions[index];
    let archived = position["journal_turns"].as_u64().unwrap_or_default();
    let mut cursor: wire::EvidenceRequest = serde_json::from_value(position["resume_history_request"].clone())
        .or_else(|_| serde_json::from_value(json!({"action":"history", "turn_start":position["resume_history_from_turn"], "include_initial":position["initial_context_archived"].as_bool().unwrap_or(true)})))?;
    for position in &positions[index + 1..] {
        let Some(results) = position["tool_results"].as_array() else {
            continue;
        };
        for result in results {
            let Some(reply) = result
                .as_str()
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
            else {
                continue;
            };
            let Ok(request) =
                serde_json::from_value::<wire::EvidenceRequest>(reply["request"].clone())
            else {
                continue;
            };
            if request.action != wire::EvidenceRequestAction::History
                || reply.get("error").is_some()
            {
                continue;
            }
            let end = request
                .turn_end
                .or_else(|| reply["total_turns"].as_u64())
                .unwrap_or_default();
            let continues = if cursor.char_start == 0 {
                request.char_start == 0
                    && request.turn_start <= cursor.turn_start
                    && end >= cursor.turn_start
                    && (!cursor.include_initial || request.include_initial)
            } else {
                request.turn_start == cursor.turn_start
                    && request.turn_end == cursor.turn_end
                    && request.include_initial == cursor.include_initial
                    && request.char_start <= cursor.char_start
                    && request.char_end.is_some_and(|end| end >= cursor.char_start)
            };
            if !continues {
                continue;
            }
            if let Ok(next) =
                serde_json::from_value::<wire::EvidenceRequest>(reply["next_request"].clone())
            {
                cursor = next;
            } else if request.char_end.is_none()
                || reply["characters"]
                    .as_u64()
                    .is_some_and(|total| request.char_end.is_some_and(|end| end >= total))
            {
                cursor.turn_start = end;
                cursor.turn_end = None;
                cursor.char_start = 0;
                cursor.char_end = None;
                cursor.include_initial &= !request.include_initial;
            } else if let Some(end) = request.char_end {
                cursor = request;
                cursor.char_start = end;
                cursor.char_end = None;
            }
        }
    }
    if !cursor.include_initial && cursor.char_start == 0 && cursor.turn_start >= archived {
        cursor.turn_start = cursor.turn_start.max(visible);
    }
    Ok(cursor)
}

pub async fn compact(
    client: &JobClient,
    mut request: wire::ModelRequest,
    journal_turns: usize,
) -> Result<Vec<wire::ModelMessage>, Error> {
    let instruction = message(wire::ModelMessageRole::User, json!({ "task": include_str!("../prompts/compact.md"), "response_schema": schema("Checkpoint")? }).to_string());
    if request.messages.is_empty() {
        request.messages.push(message(
            wire::ModelMessageRole::System,
            request.prompt.to_string(),
        ));
    }
    loop {
        let mut summarize = request.clone();
        summarize.messages.push(instruction.clone());
        match structured::<wire::Checkpoint>(client, summarize, "Checkpoint", |_| None).await {
            Ok((notes, _)) => {
                let resume = history_resume(&request.messages)?;
                return Ok(vec![
                    request.messages[0].clone(),
                    message(
                        wire::ModelMessageRole::User,
                        json!({
                            "working_notes": notes.working_notes,
                            "journal_turns": journal_turns,
                            "resume_history_from_turn": resume.turn_start,
                            "initial_context_archived": resume.include_initial,
                            "resume_history_request": resume,
                        })
                        .to_string(),
                    ),
                ]);
            }
            Err(Error::Context(_)) if request.messages.len() > 1 => {
                request
                    .messages
                    .truncate((request.messages.len() / 2).max(1));
                if request.messages.len() > 1
                    && request
                        .messages
                        .last()
                        .is_some_and(|m| m.role == wire::ModelMessageRole::Assistant)
                {
                    request.messages.pop();
                }
            }
            Err(Error::Context(_)) => {
                return Err(Error::TaskContext);
            }
            Err(error) => return Err(error),
        }
    }
}
