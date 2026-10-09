use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const DEFAULT_TIMEOUT_PER_TRIAL_MS: u64 = 1_200_000;
pub const MAX_ERROR_MESSAGE_CHARS: u64 = 2000;
pub const FINDING_ID_META_KEY: &str = "finding_id";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskCompleted {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CalledBefore {
    #[schemars(length(min = 1))]
    pub first: String,
    #[schemars(length(min = 1))]
    pub then: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Judge {
    #[schemars(length(min = 1))]
    pub prompt: String,
    #[serde(default)]
    pub model: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scorer {
    TaskCompleted(TaskCompleted),
    CalledBefore(CalledBefore),
    Judge(Judge),
}

impl Scorer {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::TaskCompleted(_) => "task_completed",
            Self::CalledBefore(_) => "called_before",
            Self::Judge(_) => "judge",
        }
    }
}

pub fn scorer_names(scorers: &[Scorer]) -> Vec<String> {
    scorers
        .iter()
        .enumerate()
        .map(|(index, scorer)| {
            let kind = scorer.kind();
            let same_kind = |other: &&Scorer| other.kind() == kind;
            if scorers.iter().filter(same_kind).count() > 1 {
                format!(
                    "{kind}_{}",
                    scorers[..index].iter().filter(same_kind).count() + 1
                )
            } else {
                kind.to_owned()
            }
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Gate {
    pub regressions: Option<u64>,
    pub critical: Option<u64>,
    #[schemars(range(min = 0.0, max = 1.0))]
    pub pass_rate: Option<f64>,
    #[schemars(range(min = 0.0))]
    pub cost_per_case: Option<f64>,
    pub min: BTreeMap<String, f64>,
}

impl Default for Gate {
    fn default() -> Self {
        Self {
            regressions: Some(0),
            critical: Some(0),
            pass_rate: None,
            cost_per_case: None,
            min: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateEvalRun {
    #[schemars(length(min = 1), regex(pattern = r"^[a-z0-9][a-z0-9_-]*$"))]
    pub eval: String,
    #[schemars(length(min = 1))]
    pub agent: String,
    pub dataset_id: String,
    #[schemars(range(min = 1))]
    pub revision: u64,
    #[serde(default)]
    pub case_ids: Option<Vec<String>>,
    #[schemars(length(min = 1))]
    pub version: String,
    #[schemars(length(min = 1))]
    pub branch: String,
    #[serde(default)]
    pub pr: Option<u64>,
    #[serde(default)]
    pub ci_url: String,
    #[serde(default = "one_trial")]
    #[schemars(range(min = 1, max = 10))]
    pub trials: u32,
    #[schemars(length(min = 1))]
    pub scorers: Vec<Scorer>,
    #[serde(default)]
    pub gate: Gate,
    #[serde(default = "default_timeout_per_trial_ms")]
    #[schemars(range(min = 1))]
    pub timeout_per_trial_ms: u64,
}

fn one_trial() -> u32 {
    1
}

fn default_timeout_per_trial_ms() -> u64 {
    DEFAULT_TIMEOUT_PER_TRIAL_MS
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum TraceAttribute {
    #[default]
    #[serde(rename = "session.id")]
    SessionId,
    #[serde(rename = "trace_id")]
    TraceId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceRef {
    #[serde(default)]
    pub attribute: TraceAttribute,
    #[schemars(length(min = 1))]
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaseError {
    pub r#type: String,
    #[schemars(length(max = 2000))]
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaseResult {
    #[serde(default)]
    pub trace: Option<TraceRef>,
    #[serde(default)]
    pub error: Option<CaseError>,
    #[serde(default)]
    #[schemars(range(min = 0.0))]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InvalidCaseResult {
    NeedsExactlyOneOfTraceOrError,
    EmptyTraceValue,
    ErrorMessageTooLong,
    NegativeOrNonFiniteCost,
}

impl CaseResult {
    pub fn validate(&self) -> Result<(), InvalidCaseResult> {
        if self.trace.is_some() == self.error.is_some() {
            return Err(InvalidCaseResult::NeedsExactlyOneOfTraceOrError);
        }
        if self
            .trace
            .as_ref()
            .is_some_and(|trace| trace.value.trim().is_empty())
        {
            return Err(InvalidCaseResult::EmptyTraceValue);
        }
        if self
            .error
            .as_ref()
            .is_some_and(|error| error.message.chars().count() as u64 > MAX_ERROR_MESSAGE_CHARS)
        {
            return Err(InvalidCaseResult::ErrorMessageTooLong);
        }
        if self
            .cost_usd
            .is_some_and(|cost| !cost.is_finite() || cost < 0.0)
        {
            return Err(InvalidCaseResult::NegativeOrNonFiniteCost);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaseDiff {
    pub case_id: String,
    pub title: String,
    pub critical: bool,
    pub baseline_url: String,
    pub candidate_url: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GateResult {
    pub passed: bool,
    #[serde(default)]
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    pub passed: u64,
    pub total: u64,
    pub pass_rate: f64,
    pub cost_per_case: f64,
    pub scores: BTreeMap<String, f64>,
    pub errors: u64,
    pub baseline_run_id: Option<String>,
    pub baseline_version: Option<String>,
    pub regressions: Vec<CaseDiff>,
    pub fixed: Vec<CaseDiff>,
    pub gate: GateResult,
}

/// One case of a run with the tool steps each trial took, for side-by-side comparison in the Runs UI
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunCase {
    pub case_id: String,
    pub title: String,
    pub critical: bool,
    pub passed: Option<bool>,
    pub trials: Vec<TrialSteps>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrialSteps {
    pub trial: u32,
    pub error: Option<String>,
    pub steps: Vec<ToolStep>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ToolStep {
    pub name: String,
    pub tool_name: String,
    pub ok: bool,
    pub start_ns: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Scoring,
    Done,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvalRun {
    pub id: String,
    pub status: RunStatus,
    pub eval: String,
    pub agent: String,
    pub version: String,
    pub branch: String,
    pub pr: Option<u64>,
    pub url: String,
    pub expected_trials: u64,
    pub received_trials: u64,
    #[serde(default)]
    pub summary: Option<Summary>,
    #[serde(default)]
    pub failure: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedDataset {
    pub id: String,
    pub name: String,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApiErrorCode {
    DatasetNotFound,
    RevisionNotFound,
    RunNotFound,
    RunClosed,
    UnknownCase,
    ContractVersion,
    Unauthorized,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApiError {
    pub detail: String,
    pub code: ApiErrorCode,
}
