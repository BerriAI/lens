use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, JsonSchema, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileSource {
    Recorded,
    Enhanced,
    Pending,
    Unknown,
    Local,
    Uncertain,
    Unclassified,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, JsonSchema, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    FunctionCalling,
    Vision,
    Reasoning,
    StructuredOutput,
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

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct MapPosition {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MapPlacement {
    Discovery,
    Categorical,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct MapPoint {
    pub trace_id: String,
    pub profile_id: String,
    pub x: f64,
    pub y: f64,
    pub placement: MapPlacement,
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
    pub center: MapPosition,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct ProfilesOverview {
    pub window: ProfileWindow,
    pub coverage: ProfileCoverage,
    pub totals: ProfileTotals,
    pub profiles: Vec<ProfileSummary>,
    pub points: Vec<MapPoint>,
    pub taxonomy: Option<TaxonomySummary>,
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityStatus {
    Compatible,
    Incompatible,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct Compatibility {
    pub profile_id: String,
    pub column_key: String,
    pub status: CompatibilityStatus,
    pub missing: Vec<Capability>,
    pub unknown: Vec<Capability>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct ProfileRef {
    pub profile_id: String,
    pub name: String,
    pub source: ProfileSource,
    /// `None` until a reviewed task definition states the requirements.
    pub required_capabilities: Option<Vec<Capability>>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct ProfilesMatrix {
    pub window: ProfileWindow,
    pub coverage: ProfileCoverage,
    pub profiles: Vec<ProfileRef>,
    pub models: Vec<ModelColumn>,
    pub cells: Vec<MatrixCell>,
    pub compatibility: Vec<Compatibility>,
    pub other_agent_requests: u64,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct TaskDefinition {
    pub task_id: String,
    pub name: String,
    pub description: String,
    pub inclusion_criteria: String,
    pub exclusion_criteria: String,
    pub required_capabilities: Vec<Capability>,
    pub example_trace_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct ProfileRunRef {
    pub trace_id: String,
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
    pub definition: Option<TaskDefinition>,
    pub runs: Vec<ProfileRunRef>,
    pub models: Vec<MatrixCell>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaxonomyStatus {
    Discovering,
    Draft,
    Frozen,
    Failed,
    Cleared,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct TaxonomySummary {
    pub taxonomy_id: String,
    pub version: u32,
    pub status: TaxonomyStatus,
    pub sample: bool,
    pub task_count: u32,
    pub frozen_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct DiscoveryPoint {
    pub team_id: String,
    pub api_key_hash: String,
    pub trace_id: String,
    pub task_id: Option<String>,
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct DiscoveryJob {
    pub job_id: String,
    pub requested_at: DateTime<Utc>,
    pub lease_until: Option<DateTime<Utc>>,
    pub attempts: u32,
    pub max_runs: u32,
    pub start_ms: i64,
    pub end_ms: i64,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct TaxonomyModels {
    pub distill: String,
    pub embedding: String,
    pub synthesis: String,
    pub decision: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct Taxonomy {
    pub taxonomy_id: String,
    pub agent: String,
    /// Generation that assignments are keyed by; it only grows, even across resets.
    pub version: u32,
    /// Changes on every write, so a stale browser edit is rejected.
    pub revision: u32,
    pub status: TaxonomyStatus,
    pub sample: bool,
    pub tasks: Vec<TaskDefinition>,
    pub points: Vec<DiscoveryPoint>,
    pub job: Option<DiscoveryJob>,
    pub discovered_at: Option<DateTime<Utc>>,
    pub frozen_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub models: TaxonomyModels,
}

impl Taxonomy {
    pub fn summary(&self) -> TaxonomySummary {
        TaxonomySummary {
            taxonomy_id: self.taxonomy_id.clone(),
            version: self.version,
            status: self.status,
            sample: self.sample,
            task_count: u32::try_from(self.tasks.len()).unwrap_or(u32::MAX),
            frozen_at: self.frozen_at,
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct TaxonomyView {
    pub taxonomy: Option<Taxonomy>,
    pub discovery_available: bool,
    pub can_write: bool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryRequest {
    pub agent: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub max_runs: u32,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaskEdit {
    pub agent: String,
    pub revision: u32,
    #[schemars(length(min = 1, max = 80))]
    pub name: String,
    #[schemars(length(max = 600))]
    pub description: String,
    #[schemars(length(max = 600))]
    pub inclusion_criteria: String,
    #[schemars(length(max = 600))]
    pub exclusion_criteria: String,
    pub required_capabilities: Vec<Capability>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TaxonomyRevisionRequest {
    pub agent: String,
    pub revision: u32,
}
