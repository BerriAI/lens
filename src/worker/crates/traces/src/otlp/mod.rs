mod attributes;
mod limits;
mod logs;
mod span;
mod wire;

pub use limits::DecodeLimits;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::{Error, NormalizedSpan, Shared};

#[derive(Serialize)]
pub struct DecodedEvent {
    pub name: String,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Serialize)]
pub struct DecodedSpan {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: String,
    pub trace_state: String,
    pub name: String,
    pub kind: String,
    pub resource_attributes: Shared<BTreeMap<String, String>>,
    pub scope_name: Shared<String>,
    pub scope_version: Shared<String>,
    pub attributes: BTreeMap<String, String>,
    pub start_ns: u64,
    pub end_ns: u64,
    pub status_code: String,
    pub status_message: String,
    pub events: Vec<DecodedEvent>,
    pub normalized: NormalizedSpan,
    pub consumed_attributes: Box<[&'static str]>,
}

#[macro_rules_attribute::apply(crate::wire_type)]
#[derive(Clone, Debug, PartialEq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum LogValue {
    Null,
    String(String),
    Boolean(bool),
    Integer(i64),
    Double(
        #[serde(
            serialize_with = "serialize_log_double",
            deserialize_with = "deserialize_log_double"
        )]
        f64,
    ),
    Bytes(Vec<u8>),
    Array(Vec<LogValue>),
    Map(BTreeMap<String, LogValue>),
}

fn serialize_log_double<S: serde::Serializer>(
    value: &f64,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    if value.is_finite() {
        serializer.serialize_f64(*value)
    } else {
        serializer.serialize_str(match *value {
            value if value.is_nan() => "NaN",
            value if value.is_sign_positive() => "Infinity",
            _ => "-Infinity",
        })
    }
}

fn deserialize_log_double<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<f64, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(value) => value
            .as_f64()
            .ok_or_else(|| serde::de::Error::custom("invalid floating-point log value")),
        serde_json::Value::String(value) => match value.as_str() {
            "NaN" => Ok(f64::NAN),
            "Infinity" => Ok(f64::INFINITY),
            "-Infinity" => Ok(f64::NEG_INFINITY),
            _ => value
                .parse()
                .map_err(|_| serde::de::Error::custom("invalid floating-point log value")),
        },
        _ => Err(serde::de::Error::custom("invalid floating-point log value")),
    }
}

#[macro_rules_attribute::apply(crate::wire_type)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CodexEventKind {
    ConversationStarts,
    StartupPhase,
    UserPrompt,
    AgentResponse,
    ToolResult,
    ToolDecision,
    SandboxOutcome,
    ApiRequest,
    SseEvent,
    TurnTtft,
    TurnCost,
    SkillInvocation,
    GuardianAssessment,
    Websocket,
    Plugin,
    Other,
}

#[macro_rules_attribute::apply(crate::wire_type)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CodexContentState {
    Present,
    Redacted,
    Absent,
    Malformed,
}

#[macro_rules_attribute::apply(crate::wire_type)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum CodexMessageRole {
    User,
    Assistant,
    Tool,
}

#[macro_rules_attribute::apply(crate::wire_type)]
#[derive(Clone, Debug)]
pub struct CodexProjection {
    pub event_kind: CodexEventKind,
    pub message_role: Option<CodexMessageRole>,
    pub content_state: Option<CodexContentState>,
    pub content: Option<String>,
    pub conversation_id: Option<String>,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
    pub call_id: Option<String>,
    pub tool_name: Option<String>,
    pub success: Option<bool>,
    pub duration_ms: Option<f64>,
    pub truncated: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DecodedLogRecord {
    pub timestamp_ns: u64,
    pub effective_timestamp_ns: u64,
    pub observed_timestamp_ns: u64,
    pub severity_number: i32,
    pub severity_text: String,
    pub event_name: String,
    pub semantic_event_name: Option<String>,
    pub body: Option<LogValue>,
    pub attributes: BTreeMap<String, LogValue>,
    pub resource_attributes: Shared<BTreeMap<String, LogValue>>,
    pub scope_name: Shared<String>,
    pub scope_version: Shared<String>,
    pub scope_attributes: Shared<BTreeMap<String, LogValue>>,
    pub resource_schema_url: String,
    pub scope_schema_url: String,
    pub trace_id: Vec<u8>,
    pub span_id: Vec<u8>,
    pub trace_flags: u32,
    pub dropped_attributes_count: u32,
    pub codex: Option<CodexProjection>,
}

#[derive(Serialize)]
pub struct DecodedLogBatch {
    pub records: Vec<DecodedLogRecord>,
    pub claude_spans: Vec<DecodedSpan>,
}

pub fn decode_otlp(body: &[u8], content_type: Option<&str>) -> Result<Vec<DecodedSpan>, Error> {
    decode_otlp_with_limits(body, content_type, DecodeLimits::from_env()?)
}

pub fn decode_otlp_with_limits(
    body: &[u8],
    content_type: Option<&str>,
    limits: DecodeLimits,
) -> Result<Vec<DecodedSpan>, Error> {
    let request = wire::decode(body, content_type, &limits)?;
    span::flatten(request, limits)
}

pub fn decode_otlp_logs(
    body: &[u8],
    content_type: Option<&str>,
) -> Result<Vec<DecodedSpan>, Error> {
    decode_otlp_logs_with_limits(body, content_type, DecodeLimits::from_env()?)
}

pub fn decode_otlp_logs_with_limits(
    body: &[u8],
    content_type: Option<&str>,
    limits: DecodeLimits,
) -> Result<Vec<DecodedSpan>, Error> {
    Ok(decode_otlp_log_batch_with_limits(body, content_type, limits)?.claude_spans)
}

pub fn decode_otlp_log_batch(
    body: &[u8],
    content_type: Option<&str>,
) -> Result<DecodedLogBatch, Error> {
    decode_otlp_log_batch_with_limits(body, content_type, DecodeLimits::from_env()?)
}

pub fn decode_otlp_log_batch_with_limits(
    body: &[u8],
    content_type: Option<&str>,
    limits: DecodeLimits,
) -> Result<DecodedLogBatch, Error> {
    logs::decode(wire::decode_logs(body, content_type, &limits)?, limits)
}
