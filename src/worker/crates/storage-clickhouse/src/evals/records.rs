use std::{collections::BTreeMap, time::Duration};

use chrono::{DateTime, Utc};
use lens_contract::eval::{CaseResult, CreateEvalRun, EvalRun, Summary};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{MAX_CASES, PAGE_SIZE};
use crate::{Error, EvalError, state::Snapshot};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredCase {
    pub id: String,
    pub title: String,
    pub critical: bool,
    pub expected: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredTrial {
    pub case_id: String,
    pub trial: u32,
    pub result: CaseResult,
    pub submitted_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredRun {
    pub team: String,
    pub request: CreateEvalRun,
    pub run: EvalRun,
    pub cases: Vec<StoredCase>,
    pub trials: Vec<StoredTrial>,
    pub verdicts: BTreeMap<String, bool>,
    pub created_at: DateTime<Utc>,
    pub scoring_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub scoring_lease: Option<ScoringLease>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScoringLease {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

pub struct RunCompletion {
    pub summary: Summary,
    pub trials: Vec<StoredTrial>,
    pub verdicts: BTreeMap<String, bool>,
}

#[derive(Clone, Debug)]
pub struct RunFilter {
    pub eval: Option<String>,
    pub agent: Option<String>,
    pub branch: Option<String>,
    pub dataset: Option<String>,
    pub after: Option<String>,
    pub limit: u32,
}

impl Default for RunFilter {
    fn default() -> Self {
        Self {
            eval: None,
            agent: None,
            branch: None,
            dataset: None,
            after: None,
            limit: 50,
        }
    }
}

pub struct ScoringPage {
    pub runs: Vec<StoredRun>,
    pub next: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct RunReference {
    pub(super) team: String,
    pub(super) id: String,
}

pub(super) fn validate_cases(
    request: &CreateEvalRun,
    cases: &[StoredCase],
) -> Result<(), EvalError> {
    if cases.is_empty() || cases.len() > MAX_CASES || !(1..=10).contains(&request.trials) {
        return Err(EvalError::InvalidRequest("invalid case or trial count"));
    }
    let unique: std::collections::BTreeSet<_> = cases.iter().map(|case| &case.id).collect();
    if unique.len() != cases.len() || unique.iter().any(|id| id.is_empty()) {
        return Err(EvalError::InvalidRequest(
            "case IDs must be unique and nonempty",
        ));
    }
    Ok(())
}

pub(super) fn validate_limit(limit: u32) -> Result<(), EvalError> {
    if !(1..=PAGE_SIZE).contains(&limit) {
        return Err(EvalError::InvalidRequest("limit must be between 1 and 100"));
    }
    Ok(())
}

pub(super) fn run_key(team: &str, id: &str) -> String {
    format!("eval-run/{}/{id}", digest(team.as_bytes()))
}

pub(super) fn scoring_key(team: &str, id: &str) -> String {
    format!("eval-scoring/{}/{id}", digest(team.as_bytes()))
}

pub(super) fn baseline_prefix(run: &StoredRun) -> Result<String, EvalError> {
    let mut scorers = run
        .request
        .scorers
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| Error::InvalidState)?;
    scorers.sort();
    let identity = serde_json::to_vec(&(
        &run.request.eval,
        &run.request.agent,
        &run.request.dataset_id,
        run.request.revision,
        scorers,
    ))
    .map_err(|_| Error::InvalidState)?;
    Ok(format!(
        "eval-done/{}/{}/",
        digest(run.team.as_bytes()),
        digest(&identity)
    ))
}

pub(super) fn descending_time(now: DateTime<Utc>) -> String {
    format!("{:020}", u64::MAX - now.timestamp_micros().max(0) as u64)
}

pub(super) fn digest(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

pub(super) fn stored_run(snapshot: &Snapshot, team: &str) -> Result<StoredRun, EvalError> {
    if snapshot.value.is_null() {
        return Err(EvalError::RunNotFound);
    }
    let run: StoredRun = decode(snapshot)?;
    if run.team != team {
        return Err(EvalError::RunNotFound);
    }
    Ok(run)
}

pub(super) fn encode(value: &impl Serialize) -> Result<serde_json::Value, EvalError> {
    serde_json::to_value(value).map_err(|_| Error::InvalidState.into())
}

pub(super) fn decode<T: serde::de::DeserializeOwned>(snapshot: &Snapshot) -> Result<T, EvalError> {
    let fields: serde_json::Map<String, serde_json::Value> =
        serde_json::from_value(snapshot.value.clone()).map_err(|_| Error::InvalidState)?;
    serde_json::from_value(serde_json::Value::Object(fields))
        .map_err(|_| Error::InvalidState.into())
}

pub(super) async fn retry(attempt: u32) {
    tokio::time::sleep(Duration::from_millis(2 * u64::from((attempt + 1).min(8)))).await;
}
