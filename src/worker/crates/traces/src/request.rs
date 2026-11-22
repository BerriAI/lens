pub const TRACE_PAGE_SIZE_MIN: u16 = 1;
pub const TRACE_PAGE_SIZE_MAX: u16 = 500;
pub const TRACE_AGENT_NAME_MAX_LENGTH: usize = 256;
pub const CONVERSATION_PAGE_SIZE_DEFAULT: u16 = 10;
pub const CONVERSATION_PAGE_SIZE_MAX: u16 = 50;
pub const LOG_PAGE_SIZE_MAX: u16 = 100;
pub const LOG_FILTER_MAX_LENGTH: usize = 256;

#[macro_rules_attribute::apply(crate::request_type)]
#[derive(Clone, Debug)]
pub struct TraceConversationRequest {
    #[serde(default)]
    pub trace_ref: String,
    #[serde(default)]
    #[cfg_attr(feature = "schema", schemars(length(max = 512)))]
    pub cursor: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "schema", schemars(range(min = TRACE_PAGE_SIZE_MIN, max = CONVERSATION_PAGE_SIZE_MAX)))]
    pub page_size: Option<u16>,
}

#[macro_rules_attribute::apply(crate::request_type)]
#[derive(Clone, Debug)]
pub struct TraceListRequest {
    /// Window start, unix ms. Default: 24h ago
    #[serde(default)]
    pub start_ms: Option<i64>,
    /// Window end, unix ms. Default: now
    #[serde(default)]
    pub end_ms: Option<i64>,
    #[serde(default)]
    #[cfg_attr(feature = "schema", schemars(length(max = TRACE_AGENT_NAME_MAX_LENGTH)))]
    pub agent: Option<String>,
    /// Also list traces that made no model call and no tool call. Default: false
    #[serde(default)]
    pub include_internal: Option<bool>,
    /// Order of the list, newest first. A cursor is only valid with the sort it was issued for. Default: start
    #[serde(default)]
    pub sort: Option<TraceSort>,
    #[serde(default)]
    #[cfg_attr(feature = "schema", schemars(length(max = 512)))]
    pub cursor: Option<String>,
}

#[macro_rules_attribute::apply(crate::request_type)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceSort {
    /// When the trace's first record started
    Start,
    /// When the trace's latest record ended
    LastActivity,
}

#[macro_rules_attribute::apply(crate::request_type)]
#[derive(Clone, Debug)]
pub struct LogListRequest {
    #[serde(default)]
    pub start_ms: Option<i64>,
    #[serde(default)]
    pub end_ms: Option<i64>,
    #[serde(default)]
    #[cfg_attr(feature = "schema", schemars(length(max = LOG_FILTER_MAX_LENGTH)))]
    pub service: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "schema", schemars(length(max = LOG_FILTER_MAX_LENGTH)))]
    pub event: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "schema", schemars(length(max = 32)))]
    pub severity: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "schema", schemars(length(max = 64)))]
    pub trace_id: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "schema", schemars(length(max = LOG_FILTER_MAX_LENGTH)))]
    pub conversation_id: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "schema", schemars(length(max = 128)))]
    pub cursor: Option<String>,
    #[serde(default)]
    #[cfg_attr(
        feature = "schema",
        schemars(range(min = TRACE_PAGE_SIZE_MIN, max = LOG_PAGE_SIZE_MAX))
    )]
    pub page_size: Option<u16>,
}

#[macro_rules_attribute::apply(crate::request_type)]
#[derive(Clone, Debug)]
pub struct TraceDetailRequest {
    #[serde(default)]
    pub trace_ref: String,
    #[serde(default)]
    #[cfg_attr(feature = "schema", schemars(length(max = 512)))]
    pub cursor: Option<String>,
    #[serde(default)]
    #[cfg_attr(
        feature = "schema",
        schemars(range(min = TRACE_PAGE_SIZE_MIN, max = TRACE_PAGE_SIZE_MAX))
    )]
    pub page_size: Option<u16>,
}

#[macro_rules_attribute::apply(crate::request_type)]
#[derive(Clone, Debug)]
pub struct TraceSpanRequest {
    #[serde(default)]
    pub trace_ref: String,
}

#[macro_rules_attribute::apply(crate::request_type)]
#[derive(Clone, Debug)]
pub struct TraceErrorPageRequest {
    #[serde(default)]
    pub trace_ref: String,
    #[serde(default)]
    #[cfg_attr(feature = "schema", schemars(length(max = 512)))]
    pub cursor: Option<String>,
}

#[macro_rules_attribute::apply(crate::request_type)]
#[derive(Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct TraceQueryRequest {
    pub sql: String,
}
