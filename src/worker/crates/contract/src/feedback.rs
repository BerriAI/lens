use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceIdentity {
    #[schemars(length(min = 1, max = 128))]
    pub trace_id: String,
    #[serde(default)]
    #[schemars(length(max = 512))]
    pub trace_ref: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FeedbackTarget {
    #[serde(default)]
    #[schemars(length(min = 1, max = 128))]
    pub trace_id: Option<String>,
    #[serde(default)]
    #[schemars(length(min = 1, max = 512))]
    pub session_id: Option<String>,
    #[serde(default)]
    #[schemars(length(max = 512))]
    pub trace_ref: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FeedbackSubmission {
    #[schemars(range(min = 0, max = 10))]
    pub score: u8,
    #[serde(default)]
    #[schemars(length(max = 10_000))]
    pub comment: String,
    #[serde(default)]
    #[schemars(length(max = 256))]
    pub user: String,
    #[serde(default)]
    #[schemars(length(min = 1, max = 128))]
    pub trace_id: Option<String>,
    #[serde(default)]
    #[schemars(length(min = 1, max = 512))]
    pub session_id: Option<String>,
    #[serde(default)]
    #[schemars(length(max = 512))]
    pub trace_ref: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FeedbackDeletion {
    #[serde(default)]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub trace_ref: String,
    #[serde(default)]
    pub user: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Feedback {
    pub trace_id: String,
    pub trace_ref: String,
    pub score: u8,
    pub comment: String,
    pub author: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceFeedback {
    pub trace_id: String,
    pub trace_ref: String,
    pub feedback: Vec<Feedback>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceFeedbackSummary {
    pub trace_id: String,
    pub trace_ref: String,
    pub count: u64,
    pub average: Option<f64>,
    pub lowest: Option<u8>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceFeedbackRequest {
    #[schemars(length(min = 1, max = 500))]
    pub traces: Vec<TraceIdentity>,
}
