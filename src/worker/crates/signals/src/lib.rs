#![forbid(unsafe_code)]

mod classifier;
mod error;
mod policy;
mod scheduler;

use chrono::{DateTime, Utc};
use lens_contract::{
    feedback::TraceIdentity,
    signals::{SignalAttempt, SignalConfig, StoredTraceSignal},
    worker::Execution,
};
use std::future::Future;

pub use classifier::{DecisionRequest, Decisions, Question, SignalState, classify, signal_state};
pub use error::{DecisionsError, Error, RepositoryError, SourceError};
pub use policy::{candidate, claimable, config_key, enabled, trace_signals};
pub use scheduler::{
    BACKLOG_SWEEP, LIVE_SWEEP, MAX_PER_TICK, SignalSweep, SignalTick, run_signal_tick,
};

pub trait SignalReader: Send + Sync {
    fn content(
        &self,
        scope: &lens_contract::investigations::Scope,
        execution: &Execution,
        cursor: &str,
    ) -> impl Future<Output = Result<lens_contract::worker::ExecutionContent, SourceError>> + Send;
    fn sample(
        &self,
        scope: &lens_contract::investigations::Scope,
        start: i64,
        end: i64,
        page_size: u32,
        cursor: &str,
    ) -> impl Future<Output = Result<lens_contract::worker::Sample, SourceError>> + Send;
}

pub trait SignalRepository: Send + Sync {
    fn get_config(&self) -> impl Future<Output = Result<SignalConfig, RepositoryError>> + Send;
    fn save_config(
        &self,
        config: &SignalConfig,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
    fn traces(
        &self,
        identities: &[TraceIdentity],
    ) -> impl Future<Output = Result<Vec<StoredTraceSignal>, RepositoryError>> + Send;
    fn claim(
        &self,
        execution: &Execution,
        config: &SignalConfig,
        claimed_until: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> impl Future<Output = Result<bool, RepositoryError>> + Send;
    fn store(
        &self,
        execution: &Execution,
        config: &SignalConfig,
        claimed_until: DateTime<Utc>,
        classified_at: DateTime<Utc>,
        attempt: &SignalAttempt,
    ) -> impl Future<Output = Result<(), RepositoryError>> + Send;
}
