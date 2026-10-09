use super::*;

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["id","phase","label","started_at"]))]
pub struct Activity {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub execution_ids: Vec<String>,
    #[serde(default)]
    #[schemars(extend("default" = false))]
    pub finished: bool,
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub operations: Vec<ActivityOperationsItem>,
    pub phase: ActivityPhase,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("default" = []))]
    pub tool_calls: Vec<ToolCount>,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    #[serde(default)]
    #[schemars(extend("default" = 0))]
    pub candidates: i64,
    #[serde(default)]
    #[schemars(extend("default" = 0))]
    pub eligible: i64,
    #[serde(default)]
    #[schemars(extend("minimum" = 0, "default" = 0))]
    pub failed_tasks: u64,
    #[serde(default)]
    #[schemars(extend("default" = 0))]
    pub grouped_batches: i64,
    #[serde(default)]
    #[schemars(extend("default" = 0))]
    pub grouping_batches: i64,
    #[serde(default)]
    #[schemars(extend("default" = 0))]
    pub inconclusive: i64,
    #[serde(default)]
    #[schemars(extend("default" = 0))]
    pub investigated: i64,
    #[serde(default)]
    #[schemars(extend("default" = 0))]
    pub partial: i64,
    #[serde(default)]
    #[schemars(extend("minimum" = 0, "default" = 0))]
    pub reusable: u64,
    #[serde(default)]
    #[schemars(extend("minimum" = 0, "default" = 0))]
    pub reused: u64,
    #[serde(default)]
    #[schemars(extend("default" = 0))]
    pub screened: i64,
    #[serde(default)]
    #[schemars(extend("default" = 0))]
    pub selected: i64,
    #[serde(default)]
    #[schemars(extend("default" = 0))]
    pub unassessable: i64,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["execution_id","trace_id","agent","started_at"]))]
pub struct InFlight {
    pub agent: String,
    pub execution_id: String,
    pub started_at: DateTime<Utc>,
    pub trace_id: String,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct Progress {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<Activity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coverage: Option<Coverage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reading: Option<Vec<InFlight>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<Review>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub at: DateTime<Utc>,
    #[serde(default)]
    #[schemars(extend("default" = 0))]
    pub completion_tokens: i64,
    #[serde(default)]
    #[schemars(extend("default" = 0))]
    pub cost: f64,
    pub kind: StepKind,
    pub label: StepLabel,
    #[serde(default)]
    #[schemars(extend("default" = ""))]
    pub model: StepModel,
    #[serde(default)]
    #[schemars(extend("default" = 0))]
    pub prompt_tokens: i64,
    #[serde(default)]
    #[schemars(extend("default" = ""))]
    pub purpose: StepPurpose,
}

#[derive(serde::Deserialize, serde::Serialize, Clone, Debug, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("required" = ["name","calls"]))]
pub struct ToolCount {
    #[schemars(extend("minimum" = 0))]
    pub calls: u64,
    pub name: ToolCountName,
}
