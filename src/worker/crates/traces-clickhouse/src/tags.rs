use std::collections::BTreeMap;

use litellm_http::Client;
use litellm_storage_clickhouse::{Query, fetch};
use litellm_traces::{
    query::named::ReadAccessParams,
    request::{TraceTag, TraceTags},
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{Config, Error, InsertTable, insert_rows};

#[derive(Serialize)]
struct TargetParams {
    #[serde(flatten)]
    access: ReadAccessParams,
    trace_id: String,
    trace_ref: String,
}

#[derive(Deserialize)]
struct TargetRow {
    team_id: String,
    key_hash: String,
    recorded: Vec<(String, String)>,
}

struct Target;
impl Query for Target {
    type Params = TargetParams;
    type Row = TargetRow;
    const SQL: &'static str = "SELECT TeamId AS team_id, ApiKeyHash AS key_hash,
         arrayFilter(t -> t.1!='', groupUniqArray(64)(tag)) AS recorded
         FROM otel_traces
         LEFT ARRAY JOIN arrayFilter(t -> length(trimBoth(t.1))>0 AND length(t.1)<=256 AND length(trimBoth(t.2))>0 AND length(t.2)<=256,
           arrayZip(mapKeys(mapConcat(ResourceAttributes,SpanAttributes)),mapValues(mapConcat(ResourceAttributes,SpanAttributes)))) AS tag
         WHERE TraceId={trace_id:String}
           AND ({trace_ref:String}='' OR hex(SHA256(concat(TeamId,char(0),ApiKeyHash,char(0),TraceId)))={trace_ref:String})
         GROUP BY TeamId,ApiKeyHash,TraceId
         HAVING {all_teams:UInt8}=1 OR has({team_ids:Array(String)},TeamId)
           OR ({user_id:String}!='' AND arraySort(groupUniqArrayIf(UserId, UserId!=''))=[{user_id:String}])
         LIMIT 2";
}

#[derive(Serialize)]
struct TagParams {
    team_id: String,
    key_hash: String,
    trace_id: String,
}

#[derive(Deserialize)]
struct TagRow {
    tags: BTreeMap<String, String>,
}

struct Manual;
impl Query for Manual {
    type Params = TagParams;
    type Row = TagRow;
    const SQL: &'static str = "SELECT Tags AS tags FROM lens_trace_tags FINAL WHERE TeamId={team_id:String} AND ApiKeyHash={key_hash:String} AND TraceId={trace_id:String} LIMIT 1";
}

async fn target(
    client: &Client,
    config: &Config,
    scope: &ReadAccessParams,
    trace_id: &str,
    trace_ref: &str,
) -> Result<Option<TargetRow>, Error> {
    let params = TargetParams {
        access: scope.clone(),
        trace_id: trace_id.into(),
        trace_ref: trace_ref.into(),
    };
    let rows = fetch::<Target>(client, config.storage().reader(), &params).await?;
    if rows.len() != 1 {
        return Ok(None);
    }
    Ok(rows.into_iter().next())
}

pub async fn read(
    client: &Client,
    config: &Config,
    scope: &ReadAccessParams,
    trace_id: &str,
    trace_ref: &str,
) -> Result<Option<TraceTags>, Error> {
    let Some(target) = target(client, config, scope, trace_id, trace_ref).await? else {
        return Ok(None);
    };
    let rows = fetch::<Manual>(
        client,
        config.storage().reader(),
        &TagParams {
            team_id: target.team_id,
            key_hash: target.key_hash,
            trace_id: trace_id.into(),
        },
    )
    .await?;
    let manual = rows
        .into_iter()
        .flat_map(|row| row.tags)
        .map(|(key, value)| TraceTag { key, value })
        .collect();
    Ok(Some(TraceTags {
        recorded: target
            .recorded
            .into_iter()
            .map(|(key, value)| TraceTag { key, value })
            .collect(),
        manual,
        editable: false,
    }))
}

pub async fn write(
    client: &Client,
    config: &Config,
    scope: &ReadAccessParams,
    trace_id: &str,
    trace_ref: &str,
    tags: &[TraceTag],
) -> Result<Option<TraceTags>, Error> {
    let Some(target) = target(client, config, scope, trace_id, trace_ref).await? else {
        return Ok(None);
    };
    let tags: BTreeMap<_, _> = tags
        .iter()
        .map(|tag| (tag.key.clone(), tag.value.clone()))
        .collect();
    insert_rows(
        client,
        config.storage().writer(),
        config.storage().database(),
        InsertTable::TraceTags,
        vec![BTreeMap::from([
            ("TeamId".into(), json!(target.team_id)),
            ("ApiKeyHash".into(), json!(target.key_hash)),
            ("TraceId".into(), json!(trace_id)),
            ("Tags".into(), json!(tags)),
            (
                "UpdatedAt".into(),
                json!(
                    time::OffsetDateTime::now_utc()
                        .format(&time::format_description::well_known::Rfc3339)
                        .map_err(|_| Error::InvalidRow)?
                ),
            ),
        ])],
    )
    .await?;
    read(client, config, scope, trace_id, trace_ref).await
}
