use std::collections::{BTreeMap, BTreeSet};

use lens_contract::eval::{TraceAttribute, TraceRef};
use litellm_http::Client;
use litellm_storage_clickhouse::{Query, fetch};
use litellm_traces::{
    SpanStatus, SpendLookup,
    query::named::{ReadAccessParams, SpendByResponseIdsParams},
};
use litellm_traces_cache::{StoreError, TraceStore};
use serde::{Deserialize, Serialize};

use crate::{ClickHouseTraces, Connection, Error};

const PAGE_SIZE: usize = 512;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EvalSpan {
    pub span_id: String,
    pub parent_span_id: String,
    pub name: String,
    #[serde(deserialize_with = "super::number::deserialize")]
    pub start_ns: i64,
    #[serde(deserialize_with = "super::number::deserialize")]
    pub end_ns: i64,
    pub status: SpanStatus,
    pub attributes: BTreeMap<String, String>,
    pub input: String,
    pub output: String,
}

#[derive(Clone, Debug)]
pub struct EvalTrace {
    pub spans: Vec<EvalSpan>,
    pub root_ended_at_ms: Option<i64>,
    pub last_received_at_ms: i64,
    pub agent: String,
    pub version: String,
    pub environment: String,
    pub gateway_cost_usd: f64,
}

#[derive(Clone)]
pub struct EvalTraces {
    client: Client,
    connection: Connection,
}

#[derive(Serialize)]
struct Lookup {
    team: String,
    attribute: &'static str,
    value: String,
    snapshot_ms: i64,
}

#[derive(Deserialize)]
struct TraceId {
    trace_id: String,
    api_key_hash: String,
}

struct Locate;

impl Query for Locate {
    type Params = Lookup;
    type Row = TraceId;
    const SQL: &'static str = "SELECT DISTINCT TraceId AS trace_id,ApiKeyHash AS api_key_hash FROM otel_traces \
        WHERE TeamId={team:String} AND EngineReceivedMs<={snapshot_ms:Int64} \
        AND (({attribute:String}='trace_id' AND TraceId={value:String}) OR \
        ({attribute:String}='session.id' \
        AND coalesce(nullIf(SpanAttributes['session.id'],''),ResourceAttributes['session.id'])={value:String})) \
        ORDER BY TraceId,ApiKeyHash";
}

#[derive(Serialize)]
struct RootLookup {
    team: String,
    trace_id: String,
    api_key_hash: String,
}

#[derive(Deserialize)]
struct RootAttributes {
    attributes: BTreeMap<String, String>,
}

struct Root;

impl Query for Root {
    type Params = RootLookup;
    type Row = RootAttributes;
    const SQL: &'static str = "SELECT mapUpdate(ResourceAttributes,SpanAttributes) AS attributes \
        FROM otel_traces WHERE TeamId={team:String} AND TraceId={trace_id:String} \
        AND ApiKeyHash={api_key_hash:String} \
        AND (ParentSpanId='' OR ParentSpanId='0000000000000000') \
        ORDER BY Timestamp,EngineReceivedMs DESC LIMIT 1";
}

#[derive(Serialize)]
struct SpanPage {
    team: String,
    trace_id: String,
    api_key_hash: String,
    after: String,
    snapshot_ms: i64,
    page_size: u32,
}

#[derive(Deserialize, Serialize)]
struct StoredSpan {
    #[serde(flatten)]
    summary: super::named::TraceSpansRow,
    trace_ref: String,
    #[serde(deserialize_with = "super::number::deserialize")]
    end_ns: i64,
    attributes: BTreeMap<String, String>,
    input: String,
    output: String,
    #[serde(deserialize_with = "super::number::deserialize")]
    received_ms: i64,
}

impl From<StoredSpan> for EvalSpan {
    fn from(row: StoredSpan) -> Self {
        let span = row.summary.0;
        Self {
            span_id: format!("{}:{}", row.trace_ref, span.span_id),
            parent_span_id: if is_root(&span.parent_span_id) {
                String::new()
            } else {
                format!("{}:{}", row.trace_ref, span.parent_span_id)
            },
            name: span.name,
            start_ns: span.start_ns,
            end_ns: row.end_ns,
            status: span.status,
            attributes: row.attributes,
            input: row.input,
            output: row.output,
        }
    }
}

struct Spans;

impl Query for Spans {
    type Params = SpanPage;
    type Row = StoredSpan;
    const SQL: &'static str = "SELECT * FROM (SELECT TraceId AS trace_id, \
        hex(SHA256(concat(TeamId,char(0),ApiKeyHash,char(0),TraceId))) AS trace_ref, \
        SpanAttributes['lens.original_trace_id'] AS original_trace_id, \
        SpanId AS span_id, ParentSpanId AS parent_span_id, \
        SpanName AS name,toUnixTimestamp64Nano(Timestamp) AS start_ns, \
        toUnixTimestamp64Nano(Timestamp)+toInt64(Duration) AS end_ns,StatusCode AS status, \
        ObservationType AS type,AgentName AS agent,Framework AS framework, \
        substringUTF8(StatusMessage,1,128) AS status_message, \
        lengthUTF8(StatusMessage)>128 AS error_truncated,Duration AS duration_ns, \
        ServiceName AS service,InputPreview AS input_preview,Model AS model, \
        InputTokens AS input_tokens,OutputTokens AS output_tokens, \
        LiteLLMRequestId AS litellm_request_id,CallKeys AS call_keys,CallEvidence AS call_evidence, \
        TeamId AS team_id,ApiKeyHash AS api_key_hash,UserId AS user_id, \
        mapUpdate(ResourceAttributes,SpanAttributes) AS attributes,Input AS input,Output AS output, \
        toInt64(EngineReceivedMs) AS received_ms \
        FROM otel_traces WHERE TeamId={team:String} AND TraceId={trace_id:String} \
        AND ApiKeyHash={api_key_hash:String} \
        AND EngineReceivedMs<={snapshot_ms:Int64} AND SpanId>{after:String} \
        ORDER BY SpanId,EngineReceivedMs DESC LIMIT 1 BY SpanId) \
        ORDER BY span_id LIMIT {page_size:UInt32}";
}

impl EvalTraces {
    pub fn new(client: Client, connection: Connection) -> Self {
        Self { client, connection }
    }

    pub async fn root_attributes(
        &self,
        team: &str,
        trace_id: &str,
    ) -> Result<BTreeMap<String, String>, Error> {
        if trace_id.is_empty() {
            return Err(Error::InvalidScope);
        }
        let traces = fetch::<Locate>(
            &self.client,
            &self.connection,
            &Lookup {
                team: team.to_owned(),
                attribute: "trace_id",
                value: trace_id.to_owned(),
                snapshot_ms: i64::MAX,
            },
        )
        .await?;
        let Some(trace) = traces.first() else {
            return Ok(BTreeMap::new());
        };
        if traces.len() != 1 {
            return Err(Error::InvalidResponse);
        }
        Ok(fetch::<Root>(
            &self.client,
            &self.connection,
            &RootLookup {
                team: team.to_owned(),
                trace_id: trace_id.to_owned(),
                api_key_hash: trace.api_key_hash.clone(),
            },
        )
        .await?
        .into_iter()
        .next()
        .map(|row| row.attributes)
        .unwrap_or_default())
    }

    pub async fn read(
        &self,
        team: &str,
        reference: &TraceRef,
        snapshot_ms: i64,
    ) -> Result<Option<EvalTrace>, Error> {
        if reference.value.is_empty() {
            return Err(Error::InvalidScope);
        }
        let lookup = Lookup {
            team: team.to_owned(),
            attribute: match reference.attribute {
                TraceAttribute::SessionId => "session.id",
                TraceAttribute::TraceId => "trace_id",
            },
            value: reference.value.clone(),
            snapshot_ms,
        };
        let traces = fetch::<Locate>(&self.client, &self.connection, &lookup).await?;
        if traces.is_empty() {
            return Ok(None);
        }
        if traces
            .iter()
            .map(|trace| &trace.api_key_hash)
            .collect::<BTreeSet<_>>()
            .len()
            != 1
        {
            return Err(Error::InvalidResponse);
        }
        let mut rows = Vec::new();
        let mut roots = Vec::new();
        let mut identities = Vec::new();
        for trace in &traces {
            let batch = self.spans(team, trace, snapshot_ms).await?;
            let root = batch
                .iter()
                .filter(|row| is_root(&row.summary.0.parent_span_id))
                .min_by_key(|row| row.summary.0.start_ns);
            roots.push(root.map(|row| (row.end_ns / 1_000_000).max(row.received_ms)));
            identities.push(
                root.or_else(|| batch.iter().min_by_key(|row| row.summary.0.start_ns))
                    .map(|row| row.attributes.clone())
                    .unwrap_or_default(),
            );
            rows.extend(batch);
        }
        let Some(last_received_at_ms) = rows.iter().map(|row| row.received_ms).max() else {
            return Ok(None);
        };
        let root_ended_at_ms = roots
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .and_then(|ended| ended.into_iter().max());
        let attribute = |name: &str| {
            let value = identities
                .first()
                .and_then(|attributes| attributes.get(name));
            if identities
                .iter()
                .all(|attributes| attributes.get(name) == value)
            {
                value.cloned().unwrap_or_default()
            } else {
                String::new()
            }
        };
        let agent = attribute("agent.name");
        let version = attribute("agent.version");
        let environment = attribute("deployment.environment");
        let keys = SpendLookup::new(
            &rows
                .iter()
                .map(|row| row.summary.0.clone())
                .collect::<Vec<_>>(),
        );
        let cost_lookup = SpendByResponseIdsParams {
            access: ReadAccessParams {
                all_teams: false,
                user_id: String::new(),
                team_ids: vec![team.to_owned()],
            },
            trace_ids: traces
                .into_iter()
                .map(|trace| trace.trace_id)
                .chain(keys.trace_ids)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            response_ids: keys.response_ids,
            provider_request_ids: keys.provider_request_ids,
            request_ids: keys.request_ids,
            start_ms: 0,
            end_ms: snapshot_ms.saturating_add(1),
        };
        let spend = ClickHouseTraces::new(self.client.clone(), self.connection.clone())
            .spend(&cost_lookup)
            .await
            .map_err(|error| match error {
                StoreError::Failed(error) => error,
                StoreError::TooLarge => {
                    Error::Storage(litellm_storage_clickhouse::Error::ResponseTooLarge)
                }
            })?;
        let owners: BTreeSet<_> = rows
            .iter()
            .map(|row| {
                (
                    row.summary.0.api_key_hash.as_str(),
                    row.summary.0.user_id.as_str(),
                )
            })
            .collect();
        let cost: f64 = spend
            .into_iter()
            .filter(|row| {
                row.team_id == team
                    && owners.iter().any(|(key, user)| {
                        (!key.is_empty() && row.api_key == *key)
                            || (!user.is_empty() && row.user == *user)
                    })
            })
            .filter_map(|row| row.spend)
            .sum();
        if !cost.is_finite() || cost < 0.0 {
            return Err(Error::InvalidResponse);
        }
        let mut spans: Vec<EvalSpan> = rows.into_iter().map(Into::into).collect();
        spans.sort_by_key(|span| span.start_ns);
        Ok(Some(EvalTrace {
            spans,
            root_ended_at_ms,
            last_received_at_ms,
            agent,
            version,
            environment,
            gateway_cost_usd: cost,
        }))
    }

    async fn spans(
        &self,
        team: &str,
        trace: &TraceId,
        snapshot_ms: i64,
    ) -> Result<Vec<StoredSpan>, Error> {
        let mut page = SpanPage {
            team: team.to_owned(),
            trace_id: trace.trace_id.clone(),
            api_key_hash: trace.api_key_hash.clone(),
            after: String::new(),
            snapshot_ms,
            page_size: PAGE_SIZE as u32,
        };
        let mut rows = Vec::new();
        loop {
            let batch = fetch::<Spans>(&self.client, &self.connection, &page).await?;
            let complete = batch.len() < PAGE_SIZE;
            if let Some(last) = batch.last() {
                page.after.clone_from(&last.summary.0.span_id);
            }
            rows.extend(batch);
            if complete {
                return Ok(rows);
            }
        }
    }
}

fn is_root(parent_span_id: &str) -> bool {
    parent_span_id.is_empty() || parent_span_id == "0000000000000000"
}
