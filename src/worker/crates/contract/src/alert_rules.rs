use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AlertRuleConfig {
    pub revision: u64,
    pub rules: Vec<AlertRule>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AlertRule {
    pub id: String,
    pub enabled: bool,
    pub condition: AlertRuleCondition,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AlertRuleCondition {
    RunDuration { minutes: u32 },
    ModelError,
}

impl AlertRuleCondition {
    pub fn kind(self) -> AlertRuleKind {
        match self {
            Self::RunDuration { .. } => AlertRuleKind::RunDuration,
            Self::ModelError => AlertRuleKind::ModelError,
        }
    }

    pub fn duration_minutes(self) -> Option<u32> {
        match self {
            Self::RunDuration { minutes } => Some(minutes),
            Self::ModelError => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AlertRuleKind {
    RunDuration,
    ModelError,
}

impl AlertRuleKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::RunDuration => "Run duration",
            Self::ModelError => "Model error",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, JsonSchema)]
pub struct AlertRuleMatch {
    pub rule_id: String,
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
