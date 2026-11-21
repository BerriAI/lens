use litellm_storage_clickhouse::Query;
use serde::{Deserialize, Serialize};

pub struct AlertRuleCandidates;

#[derive(Serialize)]
pub struct AlertRuleParams {
    pub since_ms: i64,
    pub now_ms: i64,
    pub duration_ms: u64,
    #[serde(serialize_with = "litellm_traces::wire::serialize_flag")]
    pub model_errors: bool,
    pub after: String,
    pub limit: u32,
}

#[derive(Debug, Deserialize)]
pub struct AlertRuleRow {
    pub trace_id: String,
    pub trace_ref: String,
    #[serde(deserialize_with = "super::number::deserialize")]
    pub start_ms: i64,
    #[serde(deserialize_with = "super::number::deserialize")]
    pub duration_ms: u64,
    pub duration_span_id: String,
    pub error_span_id: String,
}

impl Query for AlertRuleCandidates {
    type Params = AlertRuleParams;
    type Row = AlertRuleRow;
    const SQL: &'static str = include_str!("../../query/alert_rules.sql");
}
