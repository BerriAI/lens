use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How much of the window the figures rest on. When `truncated` is set, `runs_listed` is a
/// lower bound and the request, cost and latency figures cover only `runs_resolved` runs.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct ProfileCoverage {
    pub runs_listed: u64,
    pub runs_resolved: u64,
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct ProfileTotals {
    pub runs: u64,
    pub requests: u64,
    pub matched_requests: u64,
    pub cost_observations: u64,
    pub spend_usd: Option<f64>,
    pub other_agent_requests: u64,
    pub shared_requests: u64,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct MatrixCell {
    pub profile_id: String,
    pub column_key: String,
    pub request_count: u64,
    pub avg_cost_usd: Option<f64>,
    pub cost_observations: u64,
    pub p95_latency_ms: Option<f64>,
    pub latency_observations: u64,
    pub eval_success_rate: Option<f64>,
    pub eval_sample_count: u64,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct ProfileRunRef {
    pub trace_id: String,
    pub trace_ref: String,
    pub start_ms: i64,
    pub input_preview: String,
    pub request_count: u64,
    pub spend_usd: Option<f64>,
}
