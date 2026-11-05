use std::collections::BTreeMap;

use crate::{CodexProjection, LogValue};

#[macro_rules_attribute::apply(crate::response_type)]
#[derive(Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct TraceSQLResponse {
    #[cfg_attr(
        feature = "schema",
        schemars(extend("x-python-normalized" = {"type": "tuple[Mapping[str, JsonValue], ...]"}))
    )]
    pub data: Vec<serde_json::Map<String, serde_json::Value>>,
}

#[macro_rules_attribute::apply(crate::response_type)]
#[derive(Clone, Debug)]
pub struct OtelLogSummary {
    pub record_id: String,
    pub timestamp_ns: String,
    pub observed_timestamp_ns: String,
    pub severity_number: i32,
    pub severity_text: String,
    pub event_name: String,
    pub semantic_event_name: Option<String>,
    pub scope_name: String,
    pub service_name: String,
    pub trace_id: String,
    pub span_id: String,
    pub conversation_id: Option<String>,
}

#[macro_rules_attribute::apply(crate::response_type)]
#[derive(Clone, Debug)]
pub struct OtelLogRecord {
    pub record_id: String,
    pub timestamp_ns: String,
    pub raw_timestamp_ns: String,
    pub observed_timestamp_ns: String,
    pub severity_number: i32,
    pub severity_text: String,
    pub event_name: String,
    pub semantic_event_name: Option<String>,
    pub body: Option<LogValue>,
    pub attributes: BTreeMap<String, LogValue>,
    pub resource_attributes: BTreeMap<String, LogValue>,
    pub resource_schema_url: String,
    pub scope_name: String,
    pub scope_version: String,
    pub scope_attributes: BTreeMap<String, LogValue>,
    pub scope_schema_url: String,
    pub trace_id: String,
    pub span_id: String,
    pub trace_flags: u32,
    pub dropped_attributes_count: u32,
    pub service_name: String,
    pub conversation_id: Option<String>,
    pub codex: Option<CodexProjection>,
}

#[macro_rules_attribute::apply(crate::response_type)]
#[derive(Clone, Debug)]
pub struct OtelLogPage {
    pub data: Vec<OtelLogSummary>,
    pub next_cursor: Option<String>,
}
