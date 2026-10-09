use chrono::Utc;
use lens_contract::eval::TraceRef;
use lens_server::eval_closer::{EvalCloser, EvalCloserError, TraceSource};
use litellm_storage_clickhouse::evals::EvalStore;
use litellm_traces_clickhouse::evals::{EvalTrace, EvalTraces};

use crate::{eval_judge::GatewayJudge, eval_scoring::EvalScorer};

pub struct TraceReader(EvalTraces);

impl TraceReader {
    pub fn new(traces: EvalTraces) -> Self {
        Self(traces)
    }
}

impl TraceSource for TraceReader {
    async fn trace(
        &self,
        team: &str,
        reference: &TraceRef,
    ) -> Result<Option<EvalTrace>, EvalCloserError> {
        self.0
            .read(team, reference, Utc::now().timestamp_millis())
            .await
            .map_err(trace_error)
    }
}

fn trace_error(error: litellm_traces_clickhouse::Error) -> EvalCloserError {
    if retryable_trace_error(&error) {
        EvalCloserError::TransientTraces(Box::new(error))
    } else {
        EvalCloserError::Traces(Box::new(error))
    }
}

fn retryable_trace_error(error: &litellm_traces_clickhouse::Error) -> bool {
    use litellm_storage_clickhouse::Error as StorageError;
    use litellm_traces_clickhouse::Error;
    match error {
        Error::Busy | Error::Storage(StorageError::Transport) => true,
        Error::Storage(StorageError::QueryFailed(status)) => {
            *status == 429 || (500..600).contains(status)
        }
        Error::Cached(error) => retryable_trace_error(error),
        _ => false,
    }
}

pub fn start(
    store: EvalStore,
    traces: EvalTraces,
    judge: GatewayJudge,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(serve(store, traces, judge))
}

pub async fn serve(store: EvalStore, traces: EvalTraces, judge: GatewayJudge) {
    let closer = EvalCloser::new(store, TraceReader::new(traces), EvalScorer::new(judge));
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        if let Err(error) = closer.tick(Utc::now()).await {
            tracing::warn!(%error, "Eval closer could not complete its storage scan");
        }
    }
}

#[cfg(test)]
mod tests {
    use litellm_storage_clickhouse::Error as StorageError;
    use litellm_traces_clickhouse::Error;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::transport(Error::Storage(StorageError::Transport), true)]
    #[case::rate_limited(Error::Storage(StorageError::QueryFailed(429)), true)]
    #[case::unavailable(Error::Storage(StorageError::QueryFailed(503)), true)]
    #[case::internal(Error::Storage(StorageError::QueryFailed(500)), true)]
    #[case::concurrency_limit(Error::Busy, true)]
    #[case::cached_transport(
        Error::Cached(std::sync::Arc::new(Error::Storage(StorageError::Transport))),
        true
    )]
    #[case::invalid_response(Error::Storage(StorageError::InvalidResponse), false)]
    #[case::invalid_query(Error::Storage(StorageError::QueryFailed(400)), false)]
    #[case::unauthorized(Error::Storage(StorageError::QueryFailed(401)), false)]
    fn classifies_retryable_trace_failures(#[case] error: Error, #[case] retryable: bool) {
        assert_eq!(
            matches!(trace_error(error), EvalCloserError::TransientTraces(_)),
            retryable
        );
    }
}
