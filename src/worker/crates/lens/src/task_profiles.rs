mod calls;
mod gateway;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use lens_server::profiles::{
    ConfiguredModel, ProfileStore, ProfileStoreError, RunQuery,
    aggregate::{Run, RunCalls},
};
use litellm_traces::query::named::ReadAccessParams;
use litellm_traces_clickhouse::{
    ClickHouseTraces,
    query::profiles::{ProfileRuns, ProfileRunsParams},
};
use tokio::sync::Mutex;

use crate::{State, gateway::Models, wait_for_read_slot};

const MODEL_CACHE: Duration = Duration::from_secs(60);

struct CachedModels {
    fetched: Instant,
    models: Vec<ConfiguredModel>,
}

#[derive(Clone)]
pub struct ProfilesApi {
    state: Arc<State>,
    models: Option<Models>,
    configured: Arc<Mutex<Option<CachedModels>>>,
}

impl ProfilesApi {
    pub fn new(state: Arc<State>, models: Option<Models>) -> Self {
        Self {
            state,
            models,
            configured: Arc::new(Mutex::new(None)),
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
        let mut cached = self.configured.lock().await;
        if let Some(cached) = cached.as_ref()
            && cached.fetched.elapsed() < MODEL_CACHE
        {
            return cached.models.clone();
        }
        let fresh = gateway::configured_models(&models.client(), &connection)
            .await
            .unwrap_or_default();
        *cached = Some(CachedModels {
            fetched: Instant::now(),
            models: fresh.clone(),
        });
        fresh
    }
}
