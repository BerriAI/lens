mod calls;

use std::sync::Arc;

use lens_server::profiles::{
    ProfileStore, ProfileStoreError, RunQuery,
    aggregate::{Run, RunCalls},
};
use litellm_traces::query::named::ReadAccessParams;
use litellm_traces_clickhouse::{
    ClickHouseTraces,
    query::profiles::{ProfileRuns, ProfileRunsParams},
};

use crate::{State, wait_for_read_slot};

#[derive(Clone)]
pub struct ProfilesApi {
    state: Arc<State>,
}

impl ProfilesApi {
    pub fn new(state: Arc<State>) -> Self {
        Self { state }
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
}
