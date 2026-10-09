use chrono::Utc;
use lens_contract::eval::TraceRef;
use lens_server::eval_closer::{EvalCloser, EvalCloserError, TraceSource, UnavailableScorer};
use litellm_storage_clickhouse::evals::EvalStore;
use litellm_traces_clickhouse::evals::{EvalTrace, EvalTraces};

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
            .map_err(|error| EvalCloserError::Traces(Box::new(error)))
    }
}

pub fn start(store: EvalStore, traces: EvalTraces) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let closer = EvalCloser::new(store, TraceReader::new(traces), UnavailableScorer);
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            if let Err(error) = closer.tick(Utc::now()).await {
                tracing::warn!(%error, "Eval closer could not complete its storage scan");
            }
        }
    })
}
