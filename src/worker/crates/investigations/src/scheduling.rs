use crate::RepositoryError;
use chrono::{DateTime, Utc};
use lens_contract::investigations::{Lens, Scope, Worker};
use std::future::Future;

pub trait TraceFindingsRepository: Send + Sync {
    fn trace_findings(
        &self,
        traces: &[lens_contract::feedback::TraceIdentity],
    ) -> impl Future<
        Output = Result<Vec<lens_contract::investigations::TraceFindingCount>, RepositoryError>,
    > + Send;
}

#[derive(Clone, Debug)]
pub struct DueLens {
    pub lens: Lens,
    pub due_at: DateTime<Utc>,
}

pub trait ScheduleRepository: Send + Sync {
    fn due(
        &self,
        scope: &Scope,
        now: &DateTime<Utc>,
        limit: u32,
        after: Option<&DueLens>,
    ) -> impl Future<Output = Result<Vec<DueLens>, RepositoryError>> + Send;

    fn sync_due(&self, lens: &Lens) -> impl Future<Output = Result<(), RepositoryError>> + Send;
}

pub trait WorkerRepository: Send + Sync {
    fn workers(&self) -> impl Future<Output = Result<Vec<Worker>, RepositoryError>> + Send;

    fn eligible_workers(
        &self,
        scope: &Scope,
        after: &str,
    ) -> impl Future<Output = Result<Vec<Worker>, RepositoryError>> + Send;

    fn worker(
        &self,
        token_hash: &str,
    ) -> impl Future<Output = Result<Option<Worker>, RepositoryError>> + Send;

    fn save_worker(
        &self,
        worker: &Worker,
        token_hash: Option<&str>,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    fn configure_service_worker(
        &self,
        worker: &Worker,
        token_hash: &str,
    ) -> impl Future<Output = Result<Worker, RepositoryError>> + Send;

    fn set_worker_billing(
        &self,
        worker_id: &str,
        key_id: &str,
    ) -> impl Future<Output = Result<Option<Worker>, RepositoryError>> + Send;

    fn revoke_worker(
        &self,
        worker_id: &str,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;

    fn heartbeat(
        &self,
        worker_id: &str,
        now: DateTime<Utc>,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
}
