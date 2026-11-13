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

use lens_contract::profiles::{
    ClassificationWindow, DiscoveryModelChoice, DiscoveryModelOptions, DiscoveryPreview,
    DiscoveryPreviewRun, DiscoveryRequest, ProfileSettings, Taxonomy,
};
use lens_server::profiles::{
    ConfiguredModel, ProfileStore, ProfileStoreError, RunQuery,
    aggregate::{Run, RunCalls},
    taxonomy::{AgentState, Changes, DiscoveryCatalog, TaxonomyStore},
};
use litellm_storage_clickhouse::state::{Change, ClickHouseState, Snapshot};
use litellm_traces::query::named::ReadAccessParams;
use litellm_traces_clickhouse::{
    ClickHouseTraces,
    profiles::{BucketingParams, discovery_eligible, discovery_samples},
    query::profiles::{ProfileRuns, ProfileRunsParams},
};
use tokio::sync::Mutex;

pub use worker::ProfilesWorker;

use crate::{State, gateway::Models, wait_for_read_slot};

const MODEL_CACHE: Duration = Duration::from_secs(60);
pub(crate) const TAXONOMY_PREFIX: &str = "profiles/taxonomy/";
pub(crate) const ACTIVE_PREFIX: &str = "profiles/active/";
const SETTINGS_PREFIX: &str = "profiles/settings/";
const PREVIEW_RUNS: usize = 3;

/// Suggested aliases for the settings form; they never turn discovery on.
pub fn discovery_defaults(read: impl Fn(&str) -> Option<String>) -> DiscoveryModelChoice {
    let value = |name: &str| {
        read(name)
            .map(|value| value.trim().to_owned())
            .unwrap_or_default()
    };
    DiscoveryModelChoice {
        discovery: value("LENS_PROFILE_DISCOVERY_MODEL"),
        embedding: value("LENS_PROFILE_EMBEDDING_MODEL"),
        classification: value("LENS_PROFILE_CLASSIFICATION_MODEL"),
    }
}

pub(crate) fn ranges(windows: &[ClassificationWindow], open_end: i64) -> Vec<(i64, i64)> {
    windows
        .iter()
        .map(|window| (window.start_ms, window.end_ms.unwrap_or(open_end)))
        .collect()
}

/// Ranges whose unassigned runs still count as pending. Runs past retention are never classified,
/// so they keep their automatic group until they are deleted instead of staying pending.
pub(crate) fn pending_ranges(
    windows: &[ClassificationWindow],
    retention_days: u32,
    now_ms: i64,
) -> (Vec<i64>, Vec<i64>) {
    let cutoff = now_ms - i64::from(retention_days) * 86_400_000;
    ranges(windows, i64::MAX)
        .into_iter()
        .map(|(start, end)| (start.max(cutoff), end))
        .filter(|(start, end)| end > start)
        .unzip()
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

pub(crate) fn decode<T: serde::de::DeserializeOwned>(
    snapshot: &Snapshot,
) -> Result<Option<T>, ProfileStoreError> {
    if snapshot.value.is_null() {
        return Ok(None);
    }
    serde_json::from_value(snapshot.value.clone())
        .map(Some)
        .map_err(|_| ProfileStoreError::Unavailable)
}

pub struct Heads {
    working: Snapshot,
    active: Snapshot,
    settings: Snapshot,
}

pub(crate) async fn load_state(
    store: &ClickHouseState,
    taxonomy_id: &str,
) -> Result<(AgentState, Heads), ProfileStoreError> {
    let keys = [
        format!("{TAXONOMY_PREFIX}{taxonomy_id}"),
        format!("{ACTIVE_PREFIX}{taxonomy_id}"),
        format!("{SETTINGS_PREFIX}{taxonomy_id}"),
    ];
    let [working, active, settings]: [Snapshot; 3] = store
        .read_many(&keys.each_ref().map(String::as_str))
        .await
        .map_err(|_| ProfileStoreError::Unavailable)?
        .try_into()
        .map_err(|_| ProfileStoreError::Unavailable)?;
    Ok((
        AgentState {
            working: decode::<Taxonomy>(&working)?,
            active: decode::<Taxonomy>(&active)?,
            settings: decode::<ProfileSettings>(&settings)?,
        },
        Heads {
            working,
            active,
            settings,
        },
    ))
}

fn change<T: serde::Serialize>(
    previous: Snapshot,
    next: Option<T>,
) -> Result<Change, ProfileStoreError> {
    let value = match next {
        Some(value) => serde_json::to_value(value).map_err(|_| ProfileStoreError::Unavailable)?,
        None => previous.value.clone(),
    };
    Ok(Change { previous, value })
}

/// Commits all three documents against the revisions they were loaded at, rewriting unchanged
/// ones as they were, so a transition never lands on settings or taxonomies that moved since.
pub(crate) async fn save_state(
    store: &ClickHouseState,
    heads: Heads,
    changes: Changes,
) -> Result<(), ProfileStoreError> {
    let writes = vec![
        change(heads.working, changes.working)?,
        change(heads.active, changes.active)?,
        change(heads.settings, changes.settings)?,
    ];
    store.commit(writes).await.map_err(|error| match error {
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
        let pending = pending_ranges(
            &query.windows,
            self.state.storage.config.retention_days(),
            chrono::Utc::now().timestamp_millis(),
        );
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
                window_starts: pending.0,
                window_ends: pending.1,
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

impl TaxonomyStore for ProfilesApi {
    type Heads = Heads;

    async fn load(&self, taxonomy_id: &str) -> Result<(AgentState, Heads), ProfileStoreError> {
        load_state(&self.store, taxonomy_id).await
    }

    async fn save(&self, _: &str, heads: Heads, changes: Changes) -> Result<(), ProfileStoreError> {
        save_state(&self.store, heads, changes).await
    }
}

impl DiscoveryCatalog for ProfilesApi {
    async fn options(&self) -> DiscoveryModelOptions {
        self.models
            .as_ref()
            .map(worker::model_options)
            .unwrap_or_default()
    }

    async fn preview(
        &self,
        request: DiscoveryRequest,
    ) -> Result<DiscoveryPreview, ProfileStoreError> {
        let _permit = self.permit().await?;
        let params = BucketingParams::windows(
            &request.agent,
            &[(request.start_ms, request.end_ms)],
            request.max_runs,
        );
        let client = &self.state.storage.client;
        let config = &self.state.storage.config;
        let eligible = discovery_eligible(client, config, &params)
            .await
            .map_err(|_| ProfileStoreError::Unavailable)?;
        let sampled = discovery_samples(
            client,
            config,
            &BucketingParams {
                limit: 50,
                ..params
            },
        )
        .await
        .map_err(|_| ProfileStoreError::Unavailable)?;
        Ok(DiscoveryPreview {
            eligible_runs: eligible,
            sample_runs: u32::try_from(eligible.min(u64::from(request.max_runs)))
                .unwrap_or(request.max_runs),
            previews: sampled
                .into_iter()
                .filter(|run| !run.input_preview.trim().is_empty())
                .take(PREVIEW_RUNS)
                .map(|run| DiscoveryPreviewRun {
                    trace_id: run.trace_id,
                    start_ms: run.start_ms,
                    input_preview: run.input_preview.chars().take(300).collect(),
                })
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    const DAY: i64 = 86_400_000;
    const NOW: i64 = 100 * DAY;

    fn window(start_ms: i64, end_ms: Option<i64>) -> ClassificationWindow {
        ClassificationWindow { start_ms, end_ms }
    }

    #[rstest]
    #[case::open_window_reaching_past_retention(window(80 * DAY, None), (vec![93 * DAY], vec![i64::MAX]))]
    #[case::backfill_entirely_past_retention(window(80 * DAY, Some(90 * DAY)), (vec![], vec![]))]
    #[case::recent_backfill(window(95 * DAY, Some(96 * DAY)), (vec![95 * DAY], vec![96 * DAY]))]
    fn runs_past_retention_are_never_left_pending(
        #[case] classified: ClassificationWindow,
        #[case] expected: (Vec<i64>, Vec<i64>),
    ) {
        assert_eq!(pending_ranges(&[classified], 7, NOW), expected);
    }
}
