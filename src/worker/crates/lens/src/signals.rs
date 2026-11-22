use std::{
    sync::{Arc, Mutex, atomic::Ordering},
    time::Duration,
};

use chrono::{DateTime, Utc};
use lens_contract::{
    activity::ActivitySelection,
    feedback::TraceIdentity,
    investigations::Scope,
    signals::{SignalAttempt, SignalConfig, StoredTraceSignal},
    worker::{Execution, ExecutionContent, LensSettingsSource, Sample},
};
use lens_signals::{
    BACKLOG_SWEEP, LIVE_SWEEP, MAX_PER_TICK, RepositoryError, SignalReader, SignalRepository,
    SignalSweep, SourceError,
};
use litellm_storage_clickhouse::signals::Signals;
use tokio::time::Instant;

use crate::{Error, SampleRequest, SourceReader, wake::Wake};

mod judge;
pub use judge::SignalModels;

const CONFIG_RELOAD: Duration = Duration::from_secs(60);
const SETTLE: Duration = Duration::from_secs(16);

#[derive(Clone)]
pub struct CachedSignals<R = Signals> {
    store: R,
    config: Arc<Mutex<Option<(Instant, SignalConfig)>>>,
    wake: Wake,
    cached: bool,
}

impl<R: Clone> CachedSignals<R> {
    pub fn new(store: R, wake: Wake) -> Self {
        Self {
            store,
            config: Arc::default(),
            wake,
            cached: true,
        }
    }

    pub fn uncached(&self) -> Self {
        Self {
            cached: false,
            ..self.clone()
        }
    }

    fn remember(&self, config: &SignalConfig) {
        if let Ok(mut cache) = self.config.lock() {
            *cache = Some((Instant::now(), config.clone()));
        }
    }

    fn remembered(&self) -> Option<SignalConfig> {
        let cache = self.config.lock().ok()?;
        let (loaded, config) = cache.as_ref()?;
        (loaded.elapsed() < CONFIG_RELOAD).then(|| config.clone())
    }
}

impl<R: SignalRepository + Clone> SignalRepository for CachedSignals<R> {
    async fn get_config(&self) -> Result<SignalConfig, RepositoryError> {
        if self.cached
            && let Some(config) = self.remembered()
        {
            return Ok(config);
        }
        let config = self.store.get_config().await?;
        self.remember(&config);
        Ok(config)
    }

    async fn save_config(&self, config: &SignalConfig) -> Result<(), RepositoryError> {
        self.store.save_config(config).await?;
        self.remember(config);
        self.wake.notify();
        Ok(())
    }

    async fn traces(
        &self,
        identities: &[TraceIdentity],
    ) -> Result<Vec<StoredTraceSignal>, RepositoryError> {
        self.store.traces(identities).await
    }

    async fn recent_results(
        &self,
        config: &SignalConfig,
        since: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<StoredTraceSignal>, RepositoryError> {
        self.store.recent_results(config, since, limit).await
    }

    async fn claim(
        &self,
        execution: &Execution,
        config: &SignalConfig,
        claimed_until: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<bool, RepositoryError> {
        self.store
            .claim(execution, config, claimed_until, now)
            .await
    }

    async fn store(
        &self,
        execution: &Execution,
        config: &SignalConfig,
        claimed_until: DateTime<Utc>,
        classified_at: DateTime<Utc>,
        attempt: &SignalAttempt,
    ) -> Result<(), RepositoryError> {
        self.store
            .store(execution, config, claimed_until, classified_at, attempt)
            .await
    }
}

#[derive(Clone)]
pub struct SignalsWorker {
    pub repository: CachedSignals,
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

fn unfinished(full: bool, cursor: &str) -> bool {
    full || !cursor.is_empty()
}

impl SignalsWorker {
    async fn tick(&self, sweep: SignalSweep, cursor: &mut String) -> bool {
        let models = SignalModels {
            evaluation: self.models.evaluation(),
            analysis: self.models.analysis(),
        };
        let ready = self.sources.0.schema_ready.load(Ordering::Acquire);
        match lens_signals::run_signal_tick(
            &self.sources,
            Some(&self.repository),
            Some(&models),
            &Utc::now,
            ready,
            cursor,
            sweep,
        )
        .await
        {
            Ok(tick) => {
                *cursor = tick.cursor;
                tick.claimed >= MAX_PER_TICK
            }
            Err(error) => {
                tracing::warn!(%error,"Lens signal tick failed");
                false
            }
        }
    }

    async fn sweep(&self, sweep: SignalSweep) {
        let mut cursor = String::new();
        loop {
            let full = self.tick(sweep, &mut cursor).await;
            tokio::time::sleep(if full { Duration::ZERO } else { sweep.interval }).await;
        }
    }

    async fn live(&self) {
        let mut wake = self.sources.0.wake.listen();
        let mut cursor = String::new();
        while wake.changed().await.is_ok() {
            let mut settled = Instant::now() + SETTLE;
            let mut next = settled;
            loop {
                tokio::time::sleep_until(next).await;
                let started = Instant::now();
                if wake.has_changed().unwrap_or(false) {
                    wake.borrow_and_update();
                    settled = started + SETTLE;
                }
                let full = self.tick(LIVE_SWEEP, &mut cursor).await;
                if !unfinished(full, &cursor) && started >= settled {
                    break;
                }
                next = if full {
                    Instant::now()
                } else {
                    started + LIVE_SWEEP.interval
                };
            }
        }
    }

    pub async fn serve(self) {
        tokio::join!(self.live(), self.sweep(BACKLOG_SWEEP));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::{fixture, rstest};
    use std::sync::atomic::AtomicUsize;

    #[derive(Clone, Default)]
    struct Stored {
        config: Arc<Mutex<SignalConfig>>,
        reads: Arc<AtomicUsize>,
    }

    impl Stored {
        fn set_model(&self, model: &str) {
            self.config.lock().unwrap().model = model.into();
        }
    }

    impl SignalRepository for Stored {
        async fn get_config(&self) -> Result<SignalConfig, RepositoryError> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            Ok(self.config.lock().unwrap().clone())
        }

        async fn save_config(&self, config: &SignalConfig) -> Result<(), RepositoryError> {
            *self.config.lock().unwrap() = config.clone();
            Ok(())
        }

        async fn traces(
            &self,
            _: &[TraceIdentity],
        ) -> Result<Vec<StoredTraceSignal>, RepositoryError> {
            Ok(Vec::new())
        }

        async fn recent_results(
            &self,
            _: &SignalConfig,
            _: DateTime<Utc>,
            _: u32,
        ) -> Result<Vec<StoredTraceSignal>, RepositoryError> {
            Ok(Vec::new())
        }

        async fn claim(
            &self,
            _: &Execution,
            _: &SignalConfig,
            _: DateTime<Utc>,
            _: DateTime<Utc>,
        ) -> Result<bool, RepositoryError> {
            Ok(false)
        }

        async fn store(
            &self,
            _: &Execution,
            _: &SignalConfig,
            _: DateTime<Utc>,
            _: DateTime<Utc>,
            _: &SignalAttempt,
        ) -> Result<(), RepositoryError> {
            Ok(())
        }
    }

    struct Cache {
        stored: Stored,
        cached: CachedSignals<Stored>,
        wake: Wake,
    }

    #[fixture]
    fn cache() -> Cache {
        let stored = Stored::default();
        let wake = Wake::default();
        Cache {
            cached: CachedSignals::new(stored.clone(), wake.clone()),
            stored,
            wake,
        }
    }

    #[rstest]
    #[case::within_reload(Duration::from_secs(59), "", 1)]
    #[case::after_reload(CONFIG_RELOAD, "changed", 2)]
    #[tokio::test(start_paused = true)]
    async fn worker_rereads_settings_only_after_the_reload_period(
        cache: Cache,
        #[case] elapsed: Duration,
        #[case] model: &str,
        #[case] reads: usize,
    ) {
        cache.cached.get_config().await.unwrap();
        cache.stored.set_model("changed");
        tokio::time::advance(elapsed).await;

        assert_eq!(cache.cached.get_config().await.unwrap().model, model);
        assert_eq!(cache.stored.reads.load(Ordering::Relaxed), reads);
    }

    #[rstest]
    #[tokio::test(start_paused = true)]
    async fn saved_settings_reach_the_worker_at_once_and_wake_it(cache: Cache) {
        let woken = cache.wake.listen();
        cache.cached.get_config().await.unwrap();
        let saved = SignalConfig {
            model: "judge".into(),
            ..SignalConfig::default()
        };

        cache.cached.uncached().save_config(&saved).await.unwrap();

        assert_eq!(cache.cached.get_config().await.unwrap(), saved);
        assert_eq!(cache.stored.reads.load(Ordering::Relaxed), 1);
        assert!(woken.has_changed().unwrap());
    }

    #[rstest]
    #[tokio::test(start_paused = true)]
    async fn settings_api_always_reads_the_store(cache: Cache) {
        let api = cache.cached.uncached();
        api.get_config().await.unwrap();
        cache.stored.set_model("changed");

        assert_eq!(api.get_config().await.unwrap().model, "changed");
    }

    #[rstest]
    #[case::page_was_full(true, "", true)]
    #[case::later_pages_remain(false, "page-2", true)]
    #[case::window_is_done(false, "", false)]
    fn live_run_continues_until_the_window_is_scanned(
        #[case] full: bool,
        #[case] cursor: &str,
        #[case] expected: bool,
    ) {
        assert_eq!(unfinished(full, cursor), expected);
    }
}
