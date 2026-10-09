use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DatasetToolCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DatasetRole {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DatasetMessage {
    pub role: DatasetRole,
    pub content: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub tool_calls: Vec<DatasetToolCall>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct CaseSource {
    pub trace_id: String,
    pub trace_ref: String,
    pub span_id: String,
    pub finding_id: String,
    pub lens_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DatasetCase {
    pub id: String,
    pub messages: Vec<DatasetMessage>,
    #[serde(default)]
    pub reply: String,
    #[serde(default)]
    pub tool_calls: Vec<DatasetToolCall>,
    #[serde(default)]
    pub expected: String,
    #[serde(default = "included_by_default")]
    pub included: bool,
    pub source: CaseSource,
    #[serde(default)]
    pub agent_version: String,
}

fn included_by_default() -> bool {
    true
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    Duplicate,
    NoContent,
    TooLarge,
    OverLimit,
    Invalid,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkippedCase {
    pub source: CaseSource,
    pub reason: SkipReason,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceSource {
    #[schemars(length(min = 1))]
    pub trace_id: String,
    #[serde(default)]
    pub trace_ref: String,
    #[serde(default)]
    pub span_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindingSource {
    #[schemars(length(min = 1))]
    pub lens_id: String,
    #[schemars(length(min = 1))]
    pub finding_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TextSource {
    #[schemars(length(min = 1))]
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BuildSource {
    Trace(TraceSource),
    Finding(FindingSource),
    Text(TextSource),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BuildRequest {
    #[schemars(length(min = 1))]
    pub sources: Vec<BuildSource>,
    #[serde(default)]
    pub dataset_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BuildResult {
    pub cases: Vec<DatasetCase>,
    pub skipped: Vec<SkippedCase>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DatasetCreate {
    #[schemars(length(min = 1, max = 120))]
    pub name: String,
    #[serde(default)]
    pub agent_name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Dataset {
    pub id: String,
    pub name: String,
    pub agent_name: String,
    pub team_id: String,
    pub created_at: DateTime<Utc>,
    pub revision: i64,
    pub created_by: String,
    pub cases: Vec<DatasetCase>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DatasetSummary {
    pub id: String,
    pub name: String,
    pub agent_name: String,
    pub revision: i64,
    pub case_count: usize,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevisionSave {
    #[schemars(range(min = 0))]
    pub base_revision: i64,
    pub cases: Vec<DatasetCase>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalCases {
    pub dataset_id: String,
    pub revision: i64,
    pub cases: Vec<DatasetCase>,
}
