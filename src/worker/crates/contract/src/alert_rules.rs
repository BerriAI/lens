use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AlertRuleConfig {
    pub revision: u64,
    pub run_duration_minutes: Option<u32>,
    pub model_errors: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AlertRuleKind {
    RunDuration,
    ModelError,
}

impl AlertRuleKind {
    pub fn id(self) -> &'static str {
        match self {
            Self::RunDuration => "rule:run_duration",
            Self::ModelError => "rule:model_error",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::RunDuration => "Run duration",
            Self::ModelError => "Model error",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, JsonSchema)]
pub struct AlertRuleMatch {
    pub kind: AlertRuleKind,
    pub trace_id: String,
    pub trace_ref: String,
    pub span_id: String,
    pub detected_at: DateTime<Utc>,
    pub duration_ms: u64,
    pub threshold_minutes: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct AlertRuleOverview {
    pub since: DateTime<Utc>,
    pub limit: u32,
    pub truncated: bool,
    pub matches: Vec<AlertRuleMatch>,
}
