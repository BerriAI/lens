use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Scorer {
    TaskCompleted,
    CalledBefore {
        first: String,
        then: String,
    },
    Judge {
        prompt: String,
        #[serde(default)]
        model: String,
    },
}

impl Scorer {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::TaskCompleted => "task_completed",
            Self::CalledBefore { .. } => "called_before",
            Self::Judge { .. } => "judge",
        }
    }
}

pub fn scorer_names(scorers: &[Scorer]) -> Vec<String> {
    scorers
        .iter()
        .enumerate()
        .map(|(index, scorer)| {
            let kind = scorer.kind();
            if scorers.iter().filter(|value| value.kind() == kind).count() > 1 {
                format!(
                    "{kind}_{}",
                    scorers[..index]
                        .iter()
                        .filter(|value| value.kind() == kind)
                        .count()
                        + 1
                )
            } else {
                kind.to_owned()
            }
        })
        .collect()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Gate {
    pub regressions: Option<u64>,
    pub critical: Option<u64>,
    pub pass_rate: Option<f64>,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Case {
    pub id: String,
    pub input: String,
    pub followups: Vec<String>,
    pub meta: BTreeMap<String, String>,
    pub expected: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Source {
    #[serde(default)]
    pub finding_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DatasetCase {
    pub id: String,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub expected: String,
    #[serde(default = "included")]
    pub included: bool,
    #[serde(default)]
    pub source: Source,
    #[serde(default)]
    pub meta: BTreeMap<String, String>,
}
fn included() -> bool {
    true
}

impl DatasetCase {
    pub fn to_case(&self) -> Result<Case> {
        let messages = self
            .messages
            .iter()
            .filter(|message| message.role == "user")
            .collect::<Vec<_>>();
        let first = messages
            .first()
            .ok_or(Error::Configuration("Dataset case has no user input"))?;
        Ok(Case {
            id: self.id.clone(),
            input: first.content.clone(),
            followups: messages[1..]
                .iter()
                .map(|message| message.content.clone())
                .collect(),
            meta: self.meta.clone(),
            expected: self.expected.clone(),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvalCases {
    pub dataset_id: String,
    pub revision: u64,
    pub cases: Vec<DatasetCase>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedDataset {
    pub id: String,
    pub name: String,
    pub revision: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceRef {
    #[serde(default = "trace_attribute")]
    pub attribute: String,
    pub value: String,
}

fn trace_attribute() -> String {
    "session.id".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseError {
    pub r#type: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseResult {
    pub trace: Option<TraceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    pub error: Option<CaseError>,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<u64>,
}

impl CaseResult {
    pub fn validate(&self) -> Result<()> {
        if (self.trace.is_some() || self.output.is_some()) == self.error.is_some() {
            return Err(Error::Configuration(
                "A result requires trace or output, or an error without either",
            ));
        }
        if let Some(trace) = &self.trace
            && (!matches!(trace.attribute.as_str(), "session.id" | "trace_id")
                || trace.value.trim().is_empty())
        {
            return Err(Error::Configuration(
                "Run.trace requires one nonempty session.id or trace_id",
            ));
        }
        if self
            .cost_usd
            .is_some_and(|cost| !cost.is_finite() || cost < 0.0)
        {
            return Err(Error::Configuration(
                "Run.cost_usd must be finite and nonnegative",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateEvalRun {
    pub eval: String,
    pub agent: String,
    pub dataset_id: String,
    pub revision: u64,
    pub case_ids: Option<Vec<String>>,
    pub version: String,
    pub branch: String,
    pub pr: Option<u64>,
    #[serde(default)]
    pub ci_url: String,
    #[serde(default = "one_trial")]
    pub trials: usize,
    #[serde(default = "default_trial_timeout")]
    pub timeout_per_trial_ms: u64,
    pub scorers: Vec<Scorer>,
    #[serde(default)]
    pub gate: Gate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_io: Option<lens_contract::agent_io::AgentIo>,
}

fn one_trial() -> usize {
    1
}

fn default_trial_timeout() -> u64 {
    1_200_000
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseDiff {
    pub case_id: String,
    pub title: String,
    pub critical: bool,
    pub baseline_url: String,
    pub candidate_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateResult {
    pub passed: bool,
    #[serde(default)]
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    pub passed: usize,
    pub total: usize,
    pub pass_rate: f64,
    pub cost_per_case: f64,
    pub scores: BTreeMap<String, f64>,
    pub errors: usize,
    pub baseline_run_id: Option<String>,
    pub baseline_version: Option<String>,
    pub regressions: Vec<CaseDiff>,
    pub fixed: Vec<CaseDiff>,
    pub gate: GateResult,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Scoring,
    Done,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvalRun {
    pub id: String,
    pub status: RunStatus,
    pub eval: String,
    pub agent: String,
    pub version: String,
    pub branch: String,
    pub pr: Option<u64>,
    #[serde(default)]
    pub ci_url: String,
    pub url: String,
    pub expected_trials: usize,
    pub received_trials: usize,
    pub summary: Option<Summary>,
    #[serde(default)]
    pub failure: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrialResult {
    pub case_id: String,
    pub trial: usize,
    pub result: CaseResult,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub run: EvalRun,
    pub baseline: Option<EvalRun>,
    pub trials: Vec<TrialResult>,
}

impl Report {
    pub fn summary(&self) -> Result<&Summary> {
        if !matches!(self.run.status, RunStatus::Done) {
            return Err(Error::Infrastructure(
                "Lens returned a run without a completed summary",
            ));
        }
        self.run.summary.as_ref().ok_or(Error::Infrastructure(
            "Lens completed the run without a summary",
        ))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvalSpec {
    pub name: String,
    pub data: String,
    pub scores: Vec<Scorer>,
    pub baseline: String,
    pub trials: usize,
    pub gate: Gate,
    pub concurrency: usize,
    pub timeout_seconds: f64,
    pub finding: Option<String>,
    pub case_ids: Option<Vec<String>>,
}

impl EvalSpec {
    pub fn timeout_millis(&self) -> Result<u64> {
        let duration =
            std::time::Duration::try_from_secs_f64(self.timeout_seconds).map_err(|_| {
                Error::Configuration("timeout_per_trial must be a finite positive duration")
            })?;
        u64::try_from(duration.as_nanos().div_ceil(1_000_000))
            .ok()
            .filter(|milliseconds| *milliseconds > 0)
            .ok_or(Error::Configuration(
                "timeout_per_trial must fit a positive millisecond count",
            ))
    }

    pub fn validate(&self) -> Result<()> {
        self.timeout_millis()?;
        if self.name.is_empty()
            || !self.name.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_-".contains(&byte)
            })
            || !self.name.as_bytes()[0].is_ascii_alphanumeric()
        {
            return Err(Error::Configuration(
                "Eval name must use lowercase letters, digits, hyphens or underscores",
            ));
        }
        if self.baseline != "main" {
            return Err(Error::Configuration(
                "Contract v1 supports only baseline='main'",
            ));
        }
        if self.data.is_empty() || self.scores.is_empty() {
            return Err(Error::Configuration(
                "An eval requires a dataset and at least one scorer",
            ));
        }
        if self.trials == 0
            || self.concurrency == 0
            || !self.timeout_seconds.is_finite()
            || self.timeout_seconds <= 0.0
            || std::time::Duration::try_from_secs_f64(self.timeout_seconds + 60.0).is_err()
        {
            return Err(Error::Configuration(
                "Trials and concurrency must be positive, and timeout finite and positive",
            ));
        }
        if self.scores.iter().any(|scorer| match scorer {
            Scorer::TaskCompleted => false,
            Scorer::CalledBefore { first, then } => {
                first.trim().is_empty() || then.trim().is_empty()
            }
            Scorer::Judge { prompt, .. } => prompt.trim().is_empty(),
        }) {
            return Err(Error::Configuration(
                "Scorer tool names and judge prompts cannot be empty",
            ));
        }
        let names = scorer_names(&self.scores);
        if self.gate.min.iter().any(|(name, value)| {
            !names.contains(name) || !value.is_finite() || !(0.0..=1.0).contains(value)
        }) {
            return Err(Error::Configuration(
                "Gate.min requires declared scorers and finite scores between 0 and 1",
            ));
        }
        if self
            .gate
            .pass_rate
            .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
            || self
                .gate
                .cost_per_case
                .is_some_and(|value| !value.is_finite() || value < 0.0)
        {
            return Err(Error::Configuration(
                "Gate rates must be 0..1 and cost finite and nonnegative",
            ));
        }
        Ok(())
    }

    pub fn select(&self, dataset: &EvalCases) -> Result<Vec<Case>> {
        let included = dataset
            .cases
            .iter()
            .filter(|case| case.included)
            .collect::<Vec<_>>();
        if included
            .iter()
            .map(|case| &case.id)
            .collect::<BTreeSet<_>>()
            .len()
            != included.len()
        {
            return Err(Error::Infrastructure(
                "Lens returned duplicate dataset case IDs",
            ));
        }
        let selected = included
            .into_iter()
            .filter(|case| {
                self.case_ids
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&case.id))
                    && self.finding.as_ref().is_none_or(|finding| {
                        case.meta
                            .get("finding_id")
                            .unwrap_or(&case.source.finding_id)
                            == finding
                    })
            })
            .collect::<Vec<_>>();
        if self.case_ids.as_ref().is_some_and(|ids| {
            ids.iter()
                .any(|id| !selected.iter().any(|case| &case.id == id))
        }) {
            return Err(Error::Configuration(
                "Subset references cases not included in this dataset revision",
            ));
        }
        if selected.is_empty() {
            return Err(Error::Configuration("The eval selected no cases"));
        }
        selected.into_iter().map(DatasetCase::to_case).collect()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Execution {
    pub version: String,
    pub branch: String,
    pub pr: Option<u64>,
    pub ci_url: String,
    pub identity: String,
}

pub fn subset_name(
    name: &str,
    finding: Option<String>,
    case_ids: Option<Vec<String>>,
) -> Result<String> {
    use sha2::{Digest, Sha256};
    if finding.is_some() == case_ids.is_some() || case_ids.as_ref().is_some_and(Vec::is_empty) {
        return Err(Error::Configuration(
            "Choose exactly one nonempty subset selector",
        ));
    }
    let canonical = case_ids.map(|ids| ids.into_iter().collect::<BTreeSet<_>>());
    let selector = serde_json::to_vec(&(finding, canonical)).map_err(Error::Response)?;
    let suffix = format!("{:x}", Sha256::digest(selector));
    Ok(format!("{name}-subset-{}", &suffix[..10]))
}
