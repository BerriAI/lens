use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(
    Clone, Copy, Debug, Deserialize, Eq, Hash, JsonSchema, Ord, PartialEq, PartialOrd, Serialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ProfileSource {
    Recorded,
    Local,
    Uncertain,
    Unclassified,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct ProfileWindow {
    pub agent: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

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
pub struct ProfileSummary {
    pub profile_id: String,
    pub name: String,
    pub source: ProfileSource,
    pub run_count: u64,
    pub share: f64,
    pub request_count: u64,
    pub cost_observations: u64,
    pub spend_usd: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct ProfilesOverview {
    pub window: ProfileWindow,
    pub coverage: ProfileCoverage,
    pub totals: ProfileTotals,
    pub profiles: Vec<ProfileSummary>,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct ModelCapabilities {
    pub function_calling: Option<bool>,
    pub vision: Option<bool>,
    pub reasoning: Option<bool>,
    pub structured_output: Option<bool>,
}

/// A matrix column. `model_config_id` is the gateway deployment ID recorded on the ledger
/// row, or `None` when no matched ledger row carried one.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct ModelColumn {
    pub column_key: String,
    pub model_config_id: Option<String>,
    pub model_name: String,
    pub model_group: String,
    pub provider: String,
    pub configured: bool,
    pub capabilities: ModelCapabilities,
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
pub struct ProfileRef {
    pub profile_id: String,
    pub name: String,
    pub source: ProfileSource,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct ProfilesMatrix {
    pub window: ProfileWindow,
    pub coverage: ProfileCoverage,
    pub profiles: Vec<ProfileRef>,
    pub models: Vec<ModelColumn>,
    pub cells: Vec<MatrixCell>,
    pub other_agent_requests: u64,
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

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct ProfileDetail {
    pub window: ProfileWindow,
    pub coverage: ProfileCoverage,
    pub profile: ProfileSummary,
    pub runs: Vec<ProfileRunRef>,
    pub models: Vec<MatrixCell>,
}
