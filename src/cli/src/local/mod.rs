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
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Target {
    pub harness: String,
    pub model: String,
}

impl Target {
    pub fn key(&self) -> String {
        format!("{}/{}", self.harness, self.model)
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentResult {
    pub outcome: Outcome,
    #[serde(default)]
    pub output: String,
    #[serde(default)]
    pub cost: f64,
    #[serde(default)]
    pub judge_cost: f64,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Attempt {
    pub task_id: String,
    pub target: Target,
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
}

impl RunMeta {
    pub fn total_cases(&self) -> usize {
        self.task_count * self.targets.len()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TargetStats {
    pub target: Target,
    pub pass: u32,
    pub fail: u32,
    pub wrong_answer: u32,
    pub tool_misuse: u32,
    pub error: u32,
    pub model_cost: f64,
    pub judge_cost: f64,
    pub tool_calls_per_case: f64,
    pub repeat_rate: f64,
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

    pub fn cost(&self) -> f64 {
        self.model_cost + self.judge_cost
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub run_id: String,
    pub task_count: usize,
    pub stats: Vec<TargetStats>,
    pub recommended: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Progress {
    Started { meta: RunMeta, resumed: usize },
    Finished(Box<Attempt>),
}
