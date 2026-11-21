pub mod run;
pub mod view;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub prompt: String,
    #[serde(default)]
    pub expected: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    #[serde(default = "one")]
    pub trial: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite_id: Option<String>,
}

const fn one() -> usize {
    1
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Target {
    pub harness: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl Target {
    pub fn key(&self) -> String {
        if let Some(label) = &self.label {
            return label.clone();
        }
        match &self.model {
            Some(model) => format!("{}/{model}", self.harness),
            None => self.harness.clone(),
        }
    }

    pub fn display_name(&self) -> &str {
        self.label
            .as_deref()
            .or(self.model.as_deref())
            .unwrap_or(&self.harness)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub name: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
    #[serde(default)]
    pub error: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Outcome {
    Pass,
    Fail { reason: String },
    Error { kind: String, message: String },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub model_calls: Option<u64>,
    #[serde(default)]
    pub judge_input_tokens: Option<u64>,
    #[serde(default)]
    pub judge_output_tokens: Option<u64>,
    #[serde(default)]
    pub judge_model_calls: Option<u64>,
    #[serde(default = "legacy_metrics_complete")]
    pub metrics_complete: bool,
}

const fn legacy_metrics_complete() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentResult {
    pub outcome: Outcome,
    #[serde(default)]
    pub output: String,
    #[serde(default)]
    pub cost: Option<f64>,
    #[serde(default)]
    pub judge_cost: Option<f64>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(flatten)]
    pub metrics: Metrics,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub check_error: Option<String>,
    #[serde(default)]
    pub check_detail: Option<String>,
    #[serde(default)]
    pub duration_seconds: Option<f64>,
    #[serde(default)]
    pub run_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Attempt {
    pub task_id: String,
    pub target: Target,
    #[serde(default = "one")]
    pub trial: usize,
    pub latency_seconds: f64,
    #[serde(flatten)]
    pub result: AgentResult,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunMeta {
    pub id: String,
    pub tasks_file: String,
    pub config_file: String,
    pub targets: Vec<Target>,
    pub task_count: usize,
    pub concurrency: usize,
    #[serde(default = "one")]
    pub trials: usize,
    #[serde(default)]
    pub task_data: Option<Vec<Task>>,
    #[serde(default)]
    pub config_hash: Option<String>,
}

impl RunMeta {
    pub fn total_cases(&self) -> usize {
        self.task_count * self.targets.len() * self.trials
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TargetStats {
    pub target: Target,
    pub pass: u32,
    pub fail: u32,
    pub error: u32,
    pub model_cost: Option<f64>,
    pub judge_cost: Option<f64>,
    #[serde(flatten)]
    pub metrics: Metrics,
    pub tool_calls_per_case: Option<f64>,
    pub repeat_rate: Option<f64>,
    pub p50_seconds: f64,
    pub p95_seconds: f64,
}

impl TargetStats {
    pub fn cases(&self) -> u32 {
        self.pass + self.fail + self.error
    }

    pub fn pass_rate(&self) -> f64 {
        match self.cases() {
            0 => 0.0,
            cases => f64::from(self.pass) / f64::from(cases),
        }
    }

    pub fn cost(&self) -> Option<f64> {
        Some(self.model_cost? + self.judge_cost?)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub run_id: String,
    pub task_count: usize,
    #[serde(default = "one")]
    pub trials: usize,
    pub stats: Vec<TargetStats>,
    pub recommended: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Progress {
    Started { meta: RunMeta, resumed: usize },
    Finished(Box<Attempt>),
}
