#![forbid(unsafe_code)]

mod build;
mod cases;
mod error;

use chrono::{DateTime, Utc};
use lens_contract::datasets::{Dataset, DatasetSummary};
use litellm_traces::{SpanDetail, Trace};
use serde::{Deserialize, Serialize};
use std::future::Future;

pub use build::build_cases;
pub use cases::{
    Candidate, case_chars, case_from_span, case_id, export_jsonl, included_cases, make_case,
    revision_cases, revision_problem,
};
pub use error::{ReadError, RevisionProblem, StoreError};

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_cases: i64,
    pub max_case_chars: i64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_cases: i64::MAX,
            max_case_chars: i64::MAX,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Scope {
    pub all_teams: bool,
    pub team_id: String,
    pub api_key_hash: String,
}

pub fn can_access(viewer: &Scope, target: &Scope) -> bool {
    viewer.all_teams
        || (!target.all_teams
            && viewer.team_id == target.team_id
            && (!viewer.team_id.is_empty() || viewer.api_key_hash == target.api_key_hash))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StoredSummary {
    pub team_id: String,
    pub summary: DatasetSummary,
}

pub trait DatasetRepository: Send + Sync {
    fn summaries(&self) -> impl Future<Output = Result<Vec<StoredSummary>, StoreError>> + Send;
    fn get(
        &self,
        dataset_id: &str,
        revision: Option<i64>,
    ) -> impl Future<Output = Result<Option<Dataset>, StoreError>> + Send;
    fn insert(
        &self,
        dataset: &Dataset,
        saved_at: DateTime<Utc>,
    ) -> impl Future<Output = Result<bool, StoreError>> + Send;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Evidence {
    pub execution_id: String,
    pub span_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Finding {
    pub id: String,
    pub evidence: Vec<Evidence>,
}

pub trait DatasetReader: Send + Sync {
    fn trace(
        &self,
        trace_id: &str,
        trace_ref: &str,
    ) -> impl Future<Output = Result<Option<Trace>, ReadError>> + Send;
    fn span(
        &self,
        trace_id: &str,
        span_id: &str,
        trace_ref: &str,
    ) -> impl Future<Output = Result<Option<SpanDetail>, ReadError>> + Send;
    fn findings(
        &self,
        lens_id: &str,
        ids: &[String],
        scope: &Scope,
    ) -> impl Future<Output = Result<Vec<Finding>, ReadError>> + Send;
}
