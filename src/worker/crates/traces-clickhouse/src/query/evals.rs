use litellm_storage_clickhouse::{Query, ReadLimits};
use serde::{Deserialize, Serialize};

pub struct EvalTrace;

#[derive(Clone, Copy, Debug, Serialize)]
pub enum EvalTraceAttribute {
    #[serde(rename = "session.id")]
    SessionId,
    #[serde(rename = "trace_id")]
    TraceId,
}

#[derive(Serialize)]
pub struct EvalTraceParams {
    pub team: String,
    pub attribute: EvalTraceAttribute,
    pub value: String,
    pub cursor: String,
}

#[derive(Debug, Deserialize)]
pub struct EvalTraceRow {
    pub trace_id: String,
    pub trace_ref: String,
    pub api_key_hash: String,
    pub span_id: String,
    pub parent_span_id: String,
    pub name: String,
    pub kind: String,
    pub tool_name: String,
    pub start_ns: i64,
    pub end_ns: i64,
    pub received_ms: u64,
    pub status: litellm_traces::SpanStatus,
    pub version: String,
    pub input: String,
    pub output: String,
    pub key: String,
}

impl Query for EvalTrace {
    type Params = EvalTraceParams;
    type Row = EvalTraceRow;
    const SQL: &'static str = include_str!("../../query/eval_trace.sql");
    const READ_LIMITS: ReadLimits = ReadLimits {
        result_rows: 1000,
        response_bytes: 16 * 1024 * 1024,
        ..litellm_storage_clickhouse::READ_LIMITS
    };
}
