use crate::{Limits, RevisionProblem};
use lens_contract::datasets::{
    CaseSource, DatasetCase, DatasetMessage, DatasetRole, DatasetToolCall, SkipReason, SkippedCase,
};
use litellm_traces::{ChatRole, SpanDetail, UiContent, UiMessage};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Candidate {
    Case(DatasetCase),
    Skipped(SkippedCase),
}

#[derive(Serialize)]
struct Content<'a> {
    messages: &'a [DatasetMessage],
    reply: &'a str,
    tool_calls: &'a [DatasetToolCall],
}

fn json_text(value: &impl Serialize) -> String {
    match serde_json::to_string(value) {
        Ok(text) => text,
        Err(error) => unreachable!("dataset records contain only JSON-safe primitives: {error}"),
    }
}

pub fn case_id(messages: &[DatasetMessage], reply: &str, tool_calls: &[DatasetToolCall]) -> String {
    let content = Content {
        messages,
        reply,
        tool_calls,
    };
    format!("{:x}", Sha256::digest(json_text(&content).as_bytes()))
}

fn tool_call_chars(tool_calls: &[DatasetToolCall]) -> usize {
    tool_calls
        .iter()
        .map(|call| call.name.chars().count() + call.arguments.chars().count())
        .sum()
}

pub fn case_chars(case: &DatasetCase) -> usize {
    case.messages
        .iter()
        .map(|message| {
            message.content.chars().count()
                + message.name.chars().count()
                + tool_call_chars(&message.tool_calls)
        })
        .sum::<usize>()
        + case.reply.chars().count()
        + tool_call_chars(&case.tool_calls)
        + case.expected.chars().count()
}

pub fn make_case(
    messages: Vec<DatasetMessage>,
    reply: String,
    tool_calls: Vec<DatasetToolCall>,
    source: CaseSource,
    expected: String,
    agent_version: String,
    limits: Limits,
) -> Candidate {
    if messages.is_empty()
        && reply.trim_matches(python_whitespace).is_empty()
        && tool_calls.is_empty()
    {
        return Candidate::Skipped(SkippedCase {
            source,
            reason: SkipReason::NoContent,
        });
    }
    let case = DatasetCase {
        id: case_id(&messages, &reply, &tool_calls),
        messages,
        reply,
        tool_calls,
        expected,
        included: true,
        source,
        agent_version,
    };
    if case_chars(&case) as i128 > i128::from(limits.max_case_chars) {
        return Candidate::Skipped(SkippedCase {
            source: case.source,
            reason: SkipReason::TooLarge,
        });
    }
    Candidate::Case(case)
}

fn message(message: &UiMessage) -> DatasetMessage {
    DatasetMessage {
        role: match message.role {
            ChatRole::System => DatasetRole::System,
            ChatRole::User => DatasetRole::User,
            ChatRole::Assistant => DatasetRole::Assistant,
            ChatRole::Tool => DatasetRole::Tool,
        },
        content: message.content.clone(),
        name: message.name.clone().unwrap_or_default(),
        tool_calls: tool_calls(std::iter::once(message)),
    }
}

fn tool_calls<'a>(messages: impl Iterator<Item = &'a UiMessage>) -> Vec<DatasetToolCall> {
    messages
        .flat_map(|message| message.tool_calls.iter().flatten())
        .map(|call| DatasetToolCall {
            name: call.name.clone(),
            arguments: call.arguments.clone(),
        })
        .collect()
}

fn conversation(ui: &UiContent, raw: &str) -> Vec<DatasetMessage> {
    if let UiContent::Messages { messages } = ui {
        return messages.iter().map(message).collect();
    }
    if raw.trim_matches(python_whitespace).is_empty() {
        return Vec::new();
    }
    vec![DatasetMessage {
        role: DatasetRole::User,
        content: raw.to_owned(),
        name: String::new(),
        tool_calls: Vec::new(),
    }]
}

fn reply(ui: &UiContent, raw: &str) -> (String, Vec<DatasetToolCall>) {
    match ui {
        UiContent::Messages { messages } => {
            let assistants = messages
                .iter()
                .filter(|message| message.role == ChatRole::Assistant);
            let text = assistants
                .clone()
                .filter(|message| !message.content.is_empty())
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            (text, tool_calls(assistants))
        }
        UiContent::Text { text } => (text.clone(), Vec::new()),
        UiContent::Fields { .. } => (raw.to_owned(), Vec::new()),
    }
}

pub fn case_from_span(detail: &SpanDetail, source: CaseSource, limits: Limits) -> Candidate {
    let (text, calls) = reply(&detail.output_ui, &detail.output);
    make_case(
        conversation(&detail.input_ui, &detail.input),
        text,
        calls,
        CaseSource {
            span_id: detail.span_id.clone(),
            ..source
        },
        String::new(),
        detail
            .attributes
            .get("agent.version")
            .cloned()
            .unwrap_or_default(),
        limits,
    )
}

pub fn revision_cases(cases: &[DatasetCase]) -> Vec<DatasetCase> {
    let mut seen = HashSet::new();
    cases
        .iter()
        .filter_map(|case| {
            let id = case_id(&case.messages, &case.reply, &case.tool_calls);
            seen.insert(id.clone())
                .then(|| DatasetCase { id, ..case.clone() })
        })
        .collect()
}

pub fn revision_problem(cases: &[DatasetCase], limits: Limits) -> Option<RevisionProblem> {
    if cases.len() as i128 > i128::from(limits.max_cases) {
        return Some(RevisionProblem::TooManyCases(limits.max_cases));
    }
    cases
        .iter()
        .any(|case| case_chars(case) as i128 > i128::from(limits.max_case_chars))
        .then_some(RevisionProblem::CaseTooLarge(limits.max_case_chars))
}

pub fn included_cases(cases: &[DatasetCase]) -> Vec<DatasetCase> {
    cases.iter().filter(|case| case.included).cloned().collect()
}

pub fn export_jsonl(cases: &[DatasetCase]) -> String {
    cases
        .iter()
        .filter(|case| case.included)
        .map(|case| json_text(case) + "\n")
        .collect()
}

pub(crate) fn python_whitespace(character: char) -> bool {
    character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
}
