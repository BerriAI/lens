mod bucketing;
mod calls;
mod discovery;
mod gateway;
pub mod seed;
mod worker;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use lens_contract::profiles::{Taxonomy, TaxonomyModels};
use lens_server::profiles::{
    ConfiguredModel, ProfileStore, ProfileStoreError, RunQuery,
    aggregate::{Run, RunCalls},
    taxonomy::TaxonomyStore,
};
use litellm_storage_clickhouse::state::{Change, ClickHouseState, Snapshot};
use litellm_traces::query::named::ReadAccessParams;
use litellm_traces_clickhouse::{
    ClickHouseTraces,
    query::profiles::{ProfileRuns, ProfileRunsParams},
};
use tokio::sync::Mutex;

pub use worker::ProfilesWorker;

use crate::{State, gateway::Models, wait_for_read_slot};

const MODEL_CACHE: Duration = Duration::from_secs(60);
pub(crate) const TAXONOMY_PREFIX: &str = "profiles/taxonomy/";

pub fn discovery_models(read: impl Fn(&str) -> Option<String>) -> Option<TaxonomyModels> {
    let value = |name: &str| {
        read(name)
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    };
    Some(TaxonomyModels {
        distill: value("LENS_PROFILE_DISTILL_MODEL")?,
        embedding: value("LENS_PROFILE_EMBEDDING_MODEL")?,
        synthesis: value("LENS_PROFILE_SYNTHESIS_MODEL")?,
        decision: value("LENS_PROFILE_DECISION_MODEL")?,
    })
}

struct CachedModels {
    fetched: Instant,
    models: Vec<ConfiguredModel>,
}

#[derive(Clone)]
pub struct ProfilesApi {
    state: Arc<State>,
    store: ClickHouseState,
    models: Option<Models>,
    configured: Arc<Mutex<Option<CachedModels>>>,
}

impl ProfilesApi {
    pub fn new(state: Arc<State>, store: ClickHouseState, models: Option<Models>) -> Self {
        Self {
            state,
            store,
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

pub(crate) fn taxonomy_key(taxonomy_id: &str) -> String {
    format!("{TAXONOMY_PREFIX}{taxonomy_id}")
}

pub(crate) fn decode(snapshot: &Snapshot) -> Result<Option<Taxonomy>, ProfileStoreError> {
    if snapshot.value.is_null() {
        return Ok(None);
    }
    serde_json::from_value(snapshot.value.clone())
        .map(Some)
        .map_err(|_| ProfileStoreError::Unavailable)
}

pub(crate) async fn read_taxonomy(
    store: &ClickHouseState,
    taxonomy_id: &str,
) -> Result<(Option<Taxonomy>, Snapshot), ProfileStoreError> {
    let snapshot = store
        .read(&taxonomy_key(taxonomy_id))
        .await
        .map_err(|_| ProfileStoreError::Unavailable)?;
    Ok((decode(&snapshot)?, snapshot))
}

pub(crate) async fn write_taxonomy(
    store: &ClickHouseState,
    previous: Snapshot,
    next: Option<&Taxonomy>,
) -> Result<(), ProfileStoreError> {
    let value = match next {
        Some(taxonomy) => {
            serde_json::to_value(taxonomy).map_err(|_| ProfileStoreError::Unavailable)?
        }
        None => serde_json::Value::Null,
    };
    store
        .commit(vec![Change { previous, value }])
        .await
        .map_err(|error| match error {
            litellm_storage_clickhouse::Error::StateConflict => ProfileStoreError::Conflict,
            _ => ProfileStoreError::Unavailable,
        })
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
                taxonomy_id: query.taxonomy_id,
                taxonomy_version: query.taxonomy_version,
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

impl TaxonomyStore for ProfilesApi {
    type Head = Snapshot;

    async fn read(
        &self,
        taxonomy_id: &str,
    ) -> Result<(Option<Taxonomy>, Snapshot), ProfileStoreError> {
        read_taxonomy(&self.store, taxonomy_id).await
    }

    async fn write(&self, head: Snapshot, next: Option<Taxonomy>) -> Result<(), ProfileStoreError> {
        write_taxonomy(&self.store, head, next.as_ref()).await
    }
}
