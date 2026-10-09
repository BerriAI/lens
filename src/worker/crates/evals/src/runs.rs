use std::{collections::BTreeMap, future::Future};

use chrono::{DateTime, Utc};
use lens_contract::{
    datasets::DatasetCase,
    eval::{CaseResult, CreateEvalRun, EvalRun},
    feedback::TraceIdentity,
};
use serde::{Deserialize, Serialize};

use crate::{Evaluation, RunError};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Submission {
    pub case_id: String,
    pub trial: u32,
    pub result: CaseResult,
    pub received_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScoringLease {
    pub owner: String,
    pub until: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredRun {
    pub run: EvalRun,
    pub spec: CreateEvalRun,
    pub cases: Vec<DatasetCase>,
    pub team_id: String,
    pub version: u64,
    pub created_at: DateTime<Utc>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub completed_at: Option<DateTime<Utc>>,
    pub submissions: Vec<Submission>,
    pub verdicts: BTreeMap<String, bool>,
    #[serde(default)]
    pub resolved_traces: BTreeMap<String, Vec<TraceIdentity>>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub lease: Option<ScoringLease>,
}

pub trait RunRepository: Send + Sync {
    fn create(
        &self,
        idempotency_key: &str,
        candidate: &StoredRun,
    ) -> impl Future<Output = Result<StoredRun, RunError>> + Send;

    fn get(&self, id: &str) -> impl Future<Output = Result<Option<StoredRun>, RunError>> + Send;

    fn list(&self) -> impl Future<Output = Result<Vec<StoredRun>, RunError>> + Send;

    fn submit(
        &self,
        id: &str,
        case_id: &str,
        trial: u32,
        result: &CaseResult,
        received_at: DateTime<Utc>,
    ) -> impl Future<Output = Result<(), RunError>> + Send;

    fn finish(&self, id: &str) -> impl Future<Output = Result<EvalRun, RunError>> + Send;

    fn claim(
        &self,
        id: &str,
        owner: &str,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
    ) -> impl Future<Output = Result<Option<StoredRun>, RunError>> + Send;

    fn complete(
        &self,
        expected: &StoredRun,
        evaluation: &Evaluation,
        now: DateTime<Utc>,
    ) -> impl Future<Output = Result<(), RunError>> + Send;

    fn renew(
        &self,
        expected: &StoredRun,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
    ) -> impl Future<Output = Result<StoredRun, RunError>> + Send;

    fn fail(
        &self,
        expected: &StoredRun,
        failure: &str,
        now: DateTime<Utc>,
    ) -> impl Future<Output = Result<(), RunError>> + Send;

    fn release(&self, expected: &StoredRun) -> impl Future<Output = Result<(), RunError>> + Send;
}
