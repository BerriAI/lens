use litellm_http::Client;
use litellm_storage_clickhouse::{Query, fetch, insert_encoded_rows};
use litellm_traces::{
    TraceSummary,
    query::named::{ListTracesRow, ReadAccessParams},
};
use litellm_traces_cache::{SummaryProjection, project_run};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use crate::{ClickHouseTraces, Config, Connection, Error};

struct Summaries;

const RESOLUTION_VERSION: u32 = 1;

#[derive(Serialize)]
struct SummaryParams {
    #[serde(flatten)]
    access: ReadAccessParams,
    trace_refs: Vec<String>,
    resolution_version: u32,
}

#[derive(Deserialize)]
struct SummaryRow {
    summary: String,
}

impl Query for Summaries {
    type Params = SummaryParams;
    type Row = SummaryRow;
    const SQL: &'static str = include_str!("../query/trace_summaries.sql");
}

pub(crate) async fn read(
    client: &Client,
    connection: &Connection,
    access: &ReadAccessParams,
    runs: &[ListTracesRow],
) -> Result<Vec<TraceSummary>, Error> {
    if runs.is_empty() {
        return Ok(Vec::new());
    }
    let params = SummaryParams {
        access: access.clone(),
        trace_refs: runs.iter().map(|row| row.trace_ref.clone()).collect(),
        resolution_version: RESOLUTION_VERSION,
    };
    let rows = fetch::<Summaries>(client, connection, &params).await?;
    rows.into_iter()
        .map(|row| serde_json::from_str(&row.summary).map_err(|_| Error::InvalidResponse))
        .collect()
}

struct Candidates;

#[derive(Serialize)]
struct CandidateParams {
    resolution_version: u32,
    limit: u32,
}

#[derive(Deserialize)]
struct Candidate {
    team_id: String,
    api_key_hash: String,
    trace_id: String,
    trace_ref: String,
    #[serde(deserialize_with = "crate::query::number::deserialize")]
    source_span_count: u64,
    #[serde(deserialize_with = "crate::query::number::deserialize")]
    start_ms: i64,
}

impl Query for Candidates {
    type Params = CandidateParams;
    type Row = Candidate;
    const SQL: &'static str = include_str!("../query/trace_summary_candidates.sql");
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct Projection<'a> {
    team_id: &'a str,
    api_key_hash: &'a str,
    trace_id: &'a str,
    trace_ref: &'a str,
    scope_user_id: &'a str,
    source_span_count: u64,
    start_ts: String,
    computed_at: u64,
    resolution_version: u32,
    spend_keys: Vec<(&'static str, &'a str)>,
    summary: String,
}

fn now_ms() -> u64 {
    (OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as u64
}

fn projection<'a>(
    candidate: &'a Candidate,
    run: &'a SummaryProjection,
    now: u64,
) -> Result<Projection<'a>, Error> {
    let lookup = &run.spend_lookup;
    let spend_keys = lookup
        .response_ids
        .iter()
        .map(|key| ("provider_response", key.as_str()))
        .chain(
            lookup
                .provider_request_ids
                .iter()
                .map(|key| ("provider_request", key.as_str())),
        )
        .chain(
            lookup
                .request_ids
                .iter()
                .map(|key| ("litellm_request", key.as_str())),
        )
        .chain(
            lookup
                .trace_ids
                .iter()
                .map(|key| ("transport", key.as_str())),
        )
        .chain(
            run.canonical_request_ids
                .iter()
                .map(|key| ("canonical", key.as_str())),
        )
        .collect();
    Ok(Projection {
        team_id: &candidate.team_id,
        api_key_hash: &candidate.api_key_hash,
        trace_id: &candidate.trace_id,
        trace_ref: &candidate.trace_ref,
        scope_user_id: &run.scope_user_id,
        source_span_count: candidate.source_span_count,
        start_ts: litellm_traces::iso_time(candidate.start_ms),
        computed_at: now,
        resolution_version: RESOLUTION_VERSION,
        spend_keys,
        summary: serde_json::to_string(&run.summary).map_err(|_| Error::InvalidRow)?,
    })
}

pub async fn project(client: &Client, config: &Config, limit: u32) -> Result<usize, Error> {
    let candidates = fetch::<Candidates>(
        client,
        config.storage().reader(),
        &CandidateParams {
            resolution_version: RESOLUTION_VERSION,
            limit,
        },
    )
    .await?;
    let store = ClickHouseTraces::new(client.clone(), config.storage().reader().clone());
    let mut projections = Vec::new();
    for candidate in &candidates {
        let snapshot_ms = now_ms();
        let runs = project_run(
            &store,
            &candidate.trace_id,
            &candidate.trace_ref,
            snapshot_ms,
        )
        .await;
        let runs = match runs {
            Ok(runs) if !runs.is_empty() => runs,
            result => {
                tracing::warn!(trace_ref = %candidate.trace_ref, error = ?result.err(), "trace summary projection deferred");
                let retry = Projection {
                    team_id: &candidate.team_id,
                    api_key_hash: &candidate.api_key_hash,
                    trace_id: &candidate.trace_id,
                    trace_ref: &candidate.trace_ref,
                    scope_user_id: "",
                    source_span_count: 0,
                    start_ts: litellm_traces::iso_time(candidate.start_ms),
                    computed_at: snapshot_ms,
                    resolution_version: 0,
                    spend_keys: Vec::new(),
                    summary: String::new(),
                };
                projections.push(serde_json::to_string(&retry).map_err(|_| Error::InvalidRow)?);
                continue;
            }
        };
        for run in &runs {
            projections.push(
                serde_json::to_string(&projection(candidate, run, snapshot_ms)?)
                    .map_err(|_| Error::InvalidRow)?,
            );
        }
    }
    if !projections.is_empty() {
        let body = projections.join("\n");
        let token = format!("{:x}", Sha256::digest(body.as_bytes()));
        insert_encoded_rows(
            client,
            config.storage().writer(),
            config.storage().database(),
            "lens_trace_summaries",
            &token,
            &body,
        )
        .await?;
    }
    Ok(candidates.len())
}
