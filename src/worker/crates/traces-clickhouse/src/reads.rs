use std::collections::BTreeSet;

use litellm_http::Client;
use litellm_storage_clickhouse::{Error as StorageError, Query, fetch};
use litellm_traces::{PromptCacheUsage, Trace, TracePromptCache, query::named as contracts};
use litellm_traces_cache::{StoreError, TraceStore, spend_window};
use serde::{Deserialize, Serialize};

use crate::{
    Connection, Error,
    query::conversation::{ConversationAnchor, ConversationTurns},
    query::named::{
        ListTracesParams, ListTracesRow, SpanDetail as SpanDetailQuery, SpanError, SpanErrorParams,
        SpendByResponseIdsParams, TraceIdentity, TracePageSpansParams,
    },
};

struct RunCandidates;

const SESSION_CACHE_WINDOW_MS: i64 = 7 * 24 * 60 * 60 * 1000;

#[derive(Serialize)]
struct GatewayCacheParams<'a> {
    #[serde(flatten)]
    access: &'a contracts::ReadAccessParams,
    team: &'a str,
    owner_key: &'a str,
    owner_user: &'a str,
    snapshot_ms: u64,
    trace_ids: BTreeSet<&'a str>,
    request_ids: BTreeSet<&'a str>,
    start_ms: i64,
    end_ms: i64,
    session: String,
}

#[derive(Deserialize, Serialize)]
struct GatewayCacheRow {
    #[serde(deserialize_with = "crate::query::number::deserialize")]
    input_tokens: u64,
    #[serde(deserialize_with = "crate::query::number::deserialize")]
    read_tokens: u64,
    #[serde(deserialize_with = "crate::query::number::deserialize")]
    write_tokens: u64,
    session_id: String,
}

impl GatewayCacheRow {
    fn usage(&self) -> Option<PromptCacheUsage> {
        (self.input_tokens > 0).then_some(PromptCacheUsage {
            input_tokens: self.input_tokens,
            read_tokens: self.read_tokens,
            write_tokens: self.write_tokens,
        })
    }
}

struct GatewayCache<'a>(std::marker::PhantomData<&'a ()>);

impl<'a> Query for GatewayCache<'a> {
    type Params = GatewayCacheParams<'a>;
    type Row = GatewayCacheRow;
    const SQL: &'static str = include_str!("../query/gateway_prompt_cache.sql");
}

impl Query for RunCandidates {
    type Params = ListTracesParams;
    type Row = ListTracesRow;
    const SQL: &'static str = concat!(
        "SELECT * EXCEPT (request_ids), [] AS request_ids FROM (",
        include_str!("../query/list_traces.sql"),
        ") ORDER BY if({by_last_activity:UInt8} = 1, start_ms + duration_ms, start_ms) DESC, trace_ref DESC"
    );
}

pub struct ClickHouseTraces {
    client: Client,
    connection: Connection,
}

impl ClickHouseTraces {
    pub fn new(client: Client, connection: Connection) -> Self {
        Self { client, connection }
    }
}

impl TraceStore for ClickHouseTraces {
    type Error = Error;

    fn source(&self) -> &str {
        self.connection.url().as_str()
    }

    async fn summary_page(
        &self,
        params: &contracts::ListTracesParams,
    ) -> Result<Option<litellm_traces_cache::SummaryPage>, StoreError<Self::Error>> {
        crate::summaries::page(&self.client, &self.connection, params)
            .await
            .map_err(StoreError::Failed)
    }

    async fn summaries(
        &self,
        access: &contracts::ReadAccessParams,
        runs: &[contracts::ListTracesRow],
    ) -> Result<Vec<litellm_traces::TraceSummary>, StoreError<Self::Error>> {
        crate::summaries::read(&self.client, &self.connection, access, runs)
            .await
            .map_err(StoreError::Failed)
    }

    async fn trace_refs(
        &self,
        params: &contracts::TraceIdentityParams,
    ) -> Result<Vec<String>, StoreError<Self::Error>> {
        fetch::<TraceIdentity>(&self.client, &self.connection, params)
            .await
            .map(|rows| rows.into_iter().map(|row| row.trace_ref).collect())
            .map_err(failed)
    }

    async fn list_runs(
        &self,
        params: &contracts::ListTracesParams,
    ) -> Result<Vec<contracts::ListTracesRow>, StoreError<Self::Error>> {
        let storage_params = ListTracesParams::from(params.clone());
        match fetch::<RunCandidates>(&self.client, &self.connection, &storage_params).await {
            Ok(rows) => Ok(rows.into_iter().map(|row| row.0).collect()),
            Err(StorageError::ResponseTooLarge) => Err(StoreError::TooLarge),
            Err(error) => Err(failed(error)),
        }
    }

    async fn trace_spans(
        &self,
        params: &contracts::TraceSpansParams,
        snapshot_ms: u64,
    ) -> Result<Vec<contracts::TraceSpansRow>, StoreError<Self::Error>> {
        crate::span_batches::read_spans(&self.client, &self.connection, params.clone(), snapshot_ms)
            .await
    }

    async fn run_spans(
        &self,
        params: &contracts::TracePageSpansParams,
        snapshot_ms: u64,
    ) -> Result<Vec<contracts::TraceSpansRow>, StoreError<Self::Error>> {
        crate::span_batches::read_list_spans(
            &self.client,
            &self.connection,
            TracePageSpansParams::from(params.clone()),
            snapshot_ms,
        )
        .await
    }

    async fn spend(
        &self,
        params: &contracts::SpendByResponseIdsParams,
    ) -> Result<Vec<contracts::SpendByResponseIdsRow>, StoreError<Self::Error>> {
        crate::span_batches::read_spend(
            &self.client,
            &self.connection,
            SpendByResponseIdsParams::from(params.clone()),
        )
        .await
    }

    async fn gateway_prompt_cache(
        &self,
        access: &contracts::ReadAccessParams,
        rows: &[contracts::TraceSpansRow],
        trace: &Trace,
        snapshot_ms: u64,
    ) -> Result<TracePromptCache, StoreError<Self::Error>> {
        let Some(owner) = rows.first().filter(|row| {
            (!row.api_key_hash.is_empty() || !row.user_id.is_empty())
                && rows.iter().all(|candidate| {
                    candidate.team_id == row.team_id
                        && candidate.api_key_hash == row.api_key_hash
                        && (!row.api_key_hash.is_empty() || candidate.user_id == row.user_id)
                })
        }) else {
            return Ok(TracePromptCache::default());
        };
        let Some(window) = spend_window(rows) else {
            return Ok(TracePromptCache::default());
        };
        let params = GatewayCacheParams {
            access,
            team: &owner.team_id,
            owner_key: &owner.api_key_hash,
            owner_user: &owner.user_id,
            snapshot_ms,
            trace_ids: std::iter::once(trace.summary.trace_id.as_str())
                .chain(rows.iter().map(|row| row.original_trace_id.as_str()))
                .filter(|id| !id.is_empty())
                .collect(),
            request_ids: trace
                .spans
                .iter()
                .filter_map(|span| span.spend_log_request_id.as_deref())
                .collect(),
            start_ms: window.start,
            end_ms: window.end,
            session: String::new(),
        };
        let Some(row) = fetch::<GatewayCache<'_>>(&self.client, &self.connection, &params)
            .await
            .map_err(failed)?
            .into_iter()
            .next()
        else {
            return Ok(TracePromptCache::default());
        };
        let usage = row.usage();
        let has_session = !row.session_id.is_empty();
        let session = if !has_session {
            None
        } else {
            let params = GatewayCacheParams {
                session: row.session_id,
                start_ms: params.start_ms.saturating_sub(SESSION_CACHE_WINDOW_MS),
                end_ms: params.end_ms.saturating_add(SESSION_CACHE_WINDOW_MS),
                ..params
            };
            match fetch::<GatewayCache<'_>>(&self.client, &self.connection, &params).await {
                Ok(rows) => rows.first().and_then(GatewayCacheRow::usage),
                Err(error) => {
                    tracing::warn!(%error, "gateway session prompt cache usage unavailable");
                    None
                }
            }
        };
        Ok(TracePromptCache {
            trace: usage,
            session,
            has_session,
            grouped_session: false,
        })
    }

    async fn span_detail(
        &self,
        params: &contracts::SpanDetailParams,
    ) -> Result<Option<contracts::SpanDetailRow>, StoreError<Self::Error>> {
        match fetch::<SpanDetailQuery>(&self.client, &self.connection, params).await {
            Ok(rows) => Ok(rows.into_iter().next()),
            Err(error) => Err(failed(error)),
        }
    }

    async fn span_error(
        &self,
        params: &contracts::SpanErrorParams,
    ) -> Result<Option<contracts::SpanErrorRow>, StoreError<Self::Error>> {
        let storage_params = SpanErrorParams::from(params.clone());
        match fetch::<SpanError>(&self.client, &self.connection, &storage_params).await {
            Ok(rows) => Ok(rows.into_iter().next().map(|row| row.0)),
            Err(error) => Err(failed(error)),
        }
    }

    async fn conversation_anchor(
        &self,
        params: &contracts::TraceConversationAnchorParams,
    ) -> Result<Option<contracts::TraceConversationAnchor>, StoreError<Self::Error>> {
        fetch::<ConversationAnchor>(&self.client, &self.connection, params)
            .await
            .map(|rows| rows.into_iter().next().map(|row| row.0))
            .map_err(failed)
    }

    async fn conversation_turns(
        &self,
        params: &contracts::TraceConversationTurnsParams,
    ) -> Result<Vec<contracts::TraceConversationRow>, StoreError<Self::Error>> {
        match fetch::<ConversationTurns>(&self.client, &self.connection, params).await {
            Ok(rows) => Ok(rows.into_iter().map(|row| row.0).collect()),
            Err(StorageError::ResponseTooLarge) => Err(StoreError::TooLarge),
            Err(error) => Err(failed(error)),
        }
    }
}

fn failed(error: StorageError) -> StoreError<Error> {
    StoreError::Failed(Error::Storage(error))
}
