use std::future::Future;

use litellm_traces::query::named::ReadAccessParams;

pub use crate::error::ProfileStoreError;

pub mod aggregate;

use aggregate::{Run, RunCalls};

#[derive(Clone, Debug, PartialEq)]
pub struct RunQuery {
    pub agent: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub limit: u32,
}

pub trait ProfileStore: Send + Sync {
    fn runs(
        &self,
        scope: ReadAccessParams,
        query: RunQuery,
    ) -> impl Future<Output = Result<Vec<Run>, ProfileStoreError>> + Send;
    fn calls(
        &self,
        scope: ReadAccessParams,
        runs: Vec<Run>,
        max_spans: usize,
    ) -> impl Future<Output = Result<RunCalls, ProfileStoreError>> + Send;
}
