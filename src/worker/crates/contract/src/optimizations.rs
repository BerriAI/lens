use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OptimizationMode {
    Live,
    Shadow,
    Offline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OptimizationJudge {
    Feedback,
    Llm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OptimizationScope {
    Final,
    Agent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OptimizationStatus {
    Running,
    Paused,
    Completed,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OptimizationConfig {
    pub mode: OptimizationMode,
    pub agent: String,
    pub baseline: String,
    pub models: Vec<String>,
    pub team_id: String,
    pub traffic: u8,
    pub judge: OptimizationJudge,
    pub judge_model: String,
    pub rubric: String,
    pub baseline_sampling: u8,
    pub candidate_sampling: u8,
    pub dataset_id: String,
    pub dataset_name: String,
    pub revision: i64,
    pub scope: OptimizationScope,
    pub tolerance: f64,
}

impl OptimizationConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.agent.trim().is_empty() || self.agent.len() > 256 || self.team_id.len() > 256 {
            return Err("Choose an agent and a valid team scope");
        }
        if self.baseline.is_empty()
            || self.models.is_empty()
            || self.models.len() > 8
            || self
                .models
                .iter()
                .any(|model| model.is_empty() || model.len() > 256 || model == &self.baseline)
            || self
                .models
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.models.len()
        {
            return Err("Choose a baseline and one to eight distinct candidate models");
        }
        if !(1..=100).contains(&self.traffic)
            || !(1..=100).contains(&self.baseline_sampling)
            || !(1..=100).contains(&self.candidate_sampling)
            || !(0.0..=1.0).contains(&self.tolerance)
        {
            return Err("Invalid sampling percentage or quality tolerance");
        }
        if self.judge == OptimizationJudge::Feedback && self.mode != OptimizationMode::Live {
            return Err("Shadow and Offline require an LLM judge");
        }
        if self.judge == OptimizationJudge::Llm
            && (self.judge_model.is_empty()
                || self.rubric.trim().is_empty()
                || self.rubric.len() > 16000)
        {
            return Err("Choose a judge model and a quality rubric");
        }
        if self.mode == OptimizationMode::Offline
            && (self.dataset_id.is_empty() || self.revision < 1)
        {
            return Err("Choose a dataset revision");
        }
        if self.scope == OptimizationScope::Agent && self.mode != OptimizationMode::Offline {
            return Err("Full-agent execution is available for Offline runs");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReplayRequest {
    pub messages: Vec<Value>,
    #[serde(default)]
    pub parameters: Map<String, Value>,
}

impl ReplayRequest {
    pub fn capture(input: &str) -> Option<Self> {
        let value: Value = serde_json::from_str(input).ok()?;
        if let Some(messages) = value.as_array() {
            if messages.iter().any(|message| {
                message.get("tool_calls").is_some()
                    || message.get("role").and_then(Value::as_str) == Some("tool")
            }) {
                return None;
            }
            let request = Self {
                messages: messages.clone(),
                parameters: Map::new(),
            };
            request.validate().ok()?;
            return Some(request);
        }
        let object = value.as_object()?;
        let request = Self {
            messages: object.get("messages")?.as_array()?.clone(),
            parameters: object
                .iter()
                .filter(|(key, _)| {
                    !matches!(key.as_str(), "messages" | "model" | "stream" | "metadata")
                })
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        };
        request.validate().ok()?;
        Some(request)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.messages.is_empty()
            || serde_json::to_vec(self).map_or(true, |data| data.len() > 1_000_000)
        {
            return Err("The recorded request is empty or too large");
        }
        for message in &self.messages {
            let Some(message) = message.as_object() else {
                return Err("Invalid recorded message");
            };
            if message.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "role" | "content" | "name" | "tool_calls" | "tool_call_id"
                )
            }) {
                return Err("The recorded message format is not supported for replay");
            }
            if !matches!(
                message.get("role").and_then(Value::as_str),
                Some("system" | "developer" | "user" | "assistant" | "tool")
            ) {
                return Err("Invalid recorded message role");
            }
            if message
                .get("content")
                .is_some_and(|content| !content.is_null() && !content.is_string())
            {
                return Err("Replay currently requires text messages");
            }
            if message.get("role").and_then(Value::as_str) == Some("tool")
                && !message
                    .get("tool_call_id")
                    .is_some_and(|id| id.as_str().is_some_and(|id| !id.is_empty()))
            {
                return Err("The recorded tool result is missing its tool-call identifier");
            }
            if let Some(calls) = message.get("tool_calls") {
                let Some(calls) = calls.as_array() else {
                    return Err("Invalid recorded tool calls");
                };
                if calls.iter().any(|call| {
                    call.get("id").and_then(Value::as_str).is_none()
                        || call.get("type").and_then(Value::as_str) != Some("function")
                        || call
                            .pointer("/function/name")
                            .and_then(Value::as_str)
                            .is_none()
                        || call
                            .pointer("/function/arguments")
                            .and_then(Value::as_str)
                            .is_none()
                }) {
                    return Err("The recorded tool call is incomplete");
                }
            }
        }
        if self.parameters.keys().any(|key| {
            !matches!(
                key.as_str(),
                "temperature"
                    | "top_p"
                    | "seed"
                    | "stop"
                    | "tools"
                    | "tool_choice"
                    | "parallel_tool_calls"
                    | "response_format"
                    | "frequency_penalty"
                    | "presence_penalty"
                    | "reasoning_effort"
                    | "max_tokens"
                    | "max_completion_tokens"
            )
        }) {
            return Err("The recorded request contains unsupported parameters");
        }
        if let Some(tools) = self.parameters.get("tools") {
            let Some(tools) = tools.as_array() else {
                return Err("Invalid recorded tool definitions");
            };
            if tools.iter().any(|tool| {
                tool.get("type").and_then(Value::as_str) != Some("function")
                    || tool
                        .pointer("/function/name")
                        .and_then(Value::as_str)
                        .is_none()
            }) {
                return Err(
                    "Replay supports function tools only; hosted tools require full-agent execution",
                );
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OptimizationAssignmentRequest {
    pub agent: String,
    pub baseline: String,
    pub trace_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OptimizationAssignment {
    pub optimization_id: Option<String>,
    pub trace_id: String,
    pub model: String,
    pub baseline: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OptimizationResult {
    pub id: String,
    pub model: String,
    pub served_model: Option<String>,
    pub source_id: String,
    pub trace_id: String,
    pub span_id: String,
    pub input: String,
    pub expected: String,
    pub original: String,
    pub output: Option<String>,
    pub score: Option<f64>,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<u64>,
    pub error: Option<String>,
    pub score_error: Option<String>,
    pub completed_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OptimizationRun {
    pub id: String,
    pub config: OptimizationConfig,
    pub status: OptimizationStatus,
    pub created_at: i64,
    pub results: Vec<OptimizationResult>,
    pub pending: usize,
    pub error: Option<String>,
}
