use std::{sync::atomic::Ordering, time::Duration};

use chrono::Utc;
use lens_contract::{
    activity::ActivitySelection,
    investigations::Scope,
    worker::{Execution, ExecutionContent, LensSettingsSource, Sample},
};
use lens_signals::{
    BACKLOG_SWEEP, LIVE_SWEEP, MAX_PER_TICK, SignalReader, SignalSweep, SourceError,
};
use litellm_storage_clickhouse::signals::Signals;

use crate::{Error, SampleRequest, SourceReader};

#[derive(Clone)]
pub struct SignalsWorker {
    pub repository: Signals,
    pub sources: SourceReader,
    pub models: crate::gateway::Models,
}

impl SignalReader for SourceReader {
    async fn content(
        &self,
        scope: &Scope,
        execution: &Execution,
        cursor: &str,
    ) -> Result<ExecutionContent, SourceError> {
        SourceReader::content(self, scope, execution, cursor, None)
            .await
            .map_err(|error| SourceError(Box::new(error)))
    }

    async fn sample(
        &self,
        scope: &Scope,
        start: i64,
        end: i64,
        page_size: u32,
        cursor: &str,
    ) -> Result<Sample, SourceError> {
        let selection = ActivitySelection {
            source: LensSettingsSource::Traces,
            ..Default::default()
        };
        SourceReader::signal_sample(
            self,
            scope,
            SampleRequest {
                selection: &selection,
                start: u64::try_from(start)
                    .map_err(|_| SourceError(Box::new(Error::InvalidRequest)))?,
                end: u64::try_from(end)
                    .map_err(|_| SourceError(Box::new(Error::InvalidRequest)))?,
                offset: 0,
                page_size,
                preview: false,
                cursor,
            },
        )
        .await
        .map_err(|error| SourceError(Box::new(error)))
    }
}

impl SignalsWorker {
    async fn sweep(&self, sweep: SignalSweep) {
        let mut cursor = String::new();
        loop {
            let models = self.models.evaluation();
            let ready = self.sources.0.schema_ready.load(Ordering::Acquire);
            let delay = match lens_signals::run_signal_tick(
                &self.sources,
                Some(&self.repository),
                Some(models.as_ref()),
                &Utc::now,
                ready,
                &cursor,
                sweep,
            )
            .await
            {
                Ok(tick) => {
                    cursor = tick.cursor;
                    if tick.claimed >= MAX_PER_TICK {
                        Duration::ZERO
                    } else {
                        sweep.interval
                    }
                }
                Err(error) => {
                    tracing::warn!(%error,"Lens signal tick failed");
                    sweep.interval
                }
            };
            tokio::time::sleep(delay).await;
        }
    }

    pub async fn serve(self) {
        tokio::join!(self.sweep(LIVE_SWEEP), self.sweep(BACKLOG_SWEEP));
    }
}
