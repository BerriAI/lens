mod calls;
mod gateway;

use std::{future::Future, sync::Arc, time::Duration};

use lens_server::profiles::{
    ConfiguredModel, ProfileStore, ProfileStoreError, RunQuery,
    aggregate::{Run, RunCalls},
};
use litellm_traces::query::named::ReadAccessParams;
use litellm_traces_clickhouse::{
    ClickHouseTraces,
    query::profiles::{ProfileRuns, ProfileRunsParams},
};
use tokio::{sync::Mutex, time::Instant};

use crate::{State, gateway::Models, wait_for_read_slot};

const MODEL_CACHE: Duration = Duration::from_secs(60);

struct CachedModels {
    fetched: Instant,
    models: Vec<ConfiguredModel>,
}

/// Keeps the last successful model list for a minute. A failed fetch is not cached, and the lock
/// is never held while the gateway is called.
#[derive(Clone, Default)]
struct ModelCache(Arc<Mutex<Option<CachedModels>>>);

impl ModelCache {
    async fn get(
        &self,
        fetch: impl Future<Output = Option<Vec<ConfiguredModel>>>,
    ) -> Vec<ConfiguredModel> {
        if let Some(cached) = self.0.lock().await.as_ref()
            && cached.fetched.elapsed() < MODEL_CACHE
        {
            return cached.models.clone();
        }
        let Some(fresh) = fetch.await else {
            return Vec::new();
        };
        *self.0.lock().await = Some(CachedModels {
            fetched: Instant::now(),
            models: fresh.clone(),
        });
        fresh
    }
}

#[derive(Clone)]
pub struct ProfilesApi {
    state: Arc<State>,
    models: Option<Models>,
    configured: ModelCache,
}

impl ProfilesApi {
    pub fn new(state: Arc<State>, models: Option<Models>) -> Self {
        Self {
            state,
            models,
            configured: ModelCache::default(),
        }
    }

    fn traces(&self) -> ClickHouseTraces {
        ClickHouseTraces::new(
            self.state.storage.client.clone(),
            self.state.storage.config.storage().reader().clone(),
        )
    }

    async fn permit(&self) -> Result<tokio::sync::OwnedSemaphorePermit, ProfileStoreError> {
        self.state
            .require_storage()
            .map_err(|_| ProfileStoreError::Unavailable)?;
        wait_for_read_slot(self.state.read_slots.clone().acquire_owned())
            .await
            .map_err(|_| ProfileStoreError::Unavailable)
    }
}

impl ProfileStore for ProfilesApi {
    async fn runs(
        &self,
        scope: ReadAccessParams,
        query: RunQuery,
    ) -> Result<Vec<Run>, ProfileStoreError> {
        let _permit = self.permit().await?;
        let rows = litellm_storage_clickhouse::fetch::<ProfileRuns>(
            &self.state.storage.client,
            self.state.storage.config.storage().reader(),
            &ProfileRunsParams {
                all_teams: scope.all_teams,
                user_id: scope.user_id,
                team_ids: scope.team_ids,
                agent: query.agent,
                start_ms: query.start_ms,
                end_ms: query.end_ms,
                limit: query.limit,
            },
        )
        .await
        .map_err(|_| ProfileStoreError::Unavailable)?;
        Ok(rows
            .into_iter()
            .map(|row| Run {
                team_id: row.team_id,
                api_key_hash: row.api_key_hash,
                trace_id: row.trace_id,
                start_ms: row.start_ms,
                end_ms: row.end_ms,
                profile_id: row.profile_id,
                category: row.category,
                input_preview: row.input_preview,
                agent_count: row.agent_count,
            })
            .collect())
    }

    async fn calls(
        &self,
        scope: ReadAccessParams,
        runs: Vec<Run>,
        max_spans: usize,
    ) -> Result<RunCalls, ProfileStoreError> {
        let _permit = self.permit().await?;
        calls::resolve(
            &self.traces(),
            &self.state.storage.client,
            self.state.storage.config.storage().reader(),
            &scope,
            &runs,
            max_spans,
        )
        .await
    }

    async fn configured_models(&self) -> Vec<ConfiguredModel> {
        let Some(models) = &self.models else {
            return Vec::new();
        };
        let Some(connection) = models.connection() else {
            return Vec::new();
        };
        self.configured
            .get(gateway::configured_models(&models.client(), &connection))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::future::join;
    use lens_contract::profiles::ModelCapabilities;
    use rstest::rstest;
    use tokio::sync::oneshot;

    fn listed(model_id: &str) -> Option<Vec<ConfiguredModel>> {
        Some(vec![ConfiguredModel {
            model_id: model_id.into(),
            model_group: model_id.into(),
            provider: "openai".into(),
            capabilities: ModelCapabilities::default(),
        }])
    }

    fn ids(models: Vec<ConfiguredModel>) -> Vec<String> {
        models.into_iter().map(|model| model.model_id).collect()
    }

    #[rstest]
    #[tokio::test(start_paused = true)]
    async fn a_failed_gateway_fetch_is_not_cached() {
        let cache = ModelCache::default();

        let failed = cache.get(async { None }).await;
        let next = cache.get(async { listed("dep-1") }).await;

        assert_eq!(ids(failed), Vec::<String>::new());
        assert_eq!(ids(next), ["dep-1"]);
    }

    #[rstest]
    #[tokio::test(start_paused = true)]
    async fn a_listed_set_is_reused_for_a_minute_then_fetched_again() {
        let cache = ModelCache::default();
        cache.get(async { listed("dep-1") }).await;

        let reused = cache.get(async { listed("dep-2") }).await;
        tokio::time::advance(MODEL_CACHE).await;
        let refreshed = cache.get(async { listed("dep-3") }).await;

        assert_eq!(ids(reused), ["dep-1"]);
        assert_eq!(ids(refreshed), ["dep-3"]);
    }

    #[rstest]
    #[tokio::test(start_paused = true)]
    async fn a_slow_gateway_fetch_does_not_block_other_readers() {
        let cache = ModelCache::default();
        let (release, gate) = oneshot::channel::<()>();
        let slow = cache.get(async {
            gate.await.ok();
            listed("slow")
        });
        let fast = async {
            let models = cache.get(async { listed("fast") }).await;
            release.send(()).ok();
            models
        };

        let (slow, fast) = tokio::time::timeout(Duration::from_secs(5), join(slow, fast))
            .await
            .expect("a reader waited on the in-flight fetch");

        assert_eq!(ids(slow), ["slow"]);
        assert_eq!(ids(fast), ["fast"]);
    }
}
