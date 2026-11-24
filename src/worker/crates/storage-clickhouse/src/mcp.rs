use chrono::{DateTime, Utc};
use lens_auth::{
    McpError, StoreError,
    mcp::{ActivityQuery, CONNECTION_LIMIT, McpRepository, StoredConnection},
};
use lens_contract::mcp::{McpActivity, McpCall, McpUsage};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    Error,
    sessions::Sessions,
    state::{Change, ClickHouseState},
};

const CATALOG: &str = "mcp/connections";

fn failure(error: Error) -> StoreError {
    match error {
        Error::StateConflict | Error::StateExists => StoreError::Conflict,
        error => StoreError::Unavailable(Box::new(error)),
    }
}

fn decode<T: serde::de::DeserializeOwned>(text: &str) -> Result<Vec<T>, StoreError> {
    text.lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            serde_json::from_str(line).map_err(|error| StoreError::Unavailable(Box::new(error)))
        })
        .collect()
}

fn records(value: Value) -> Result<Vec<StoredConnection>, StoreError> {
    if value.is_null() {
        return Ok(Vec::new());
    }
    serde_json::from_value(value).map_err(|error| StoreError::Unavailable(Box::new(error)))
}

pub(crate) async fn initialize(state: &ClickHouseState) -> Result<(), Error> {
    state.command(
        "CREATE TABLE IF NOT EXISTS lens_mcp_calls \
         (id String, at_ms Int64, owner_id String, connection_id String, outcome String, data String CODEC(ZSTD(3))) \
         ENGINE=MergeTree ORDER BY (owner_id, at_ms, id) \
         TTL toDateTime(intDiv(at_ms, 1000)) + INTERVAL 30 DAY \
         SETTINGS index_granularity=8192",
        &[], String::new(),
    ).await?;
    Ok(())
}

impl McpRepository for Sessions {
    async fn mcp_connections(&self) -> Result<Vec<StoredConnection>, StoreError> {
        records(self.0.read(CATALOG).await.map_err(failure)?.value)
    }

    async fn mcp_insert(&self, record: &StoredConnection) -> Result<(), McpError> {
        for attempt in 0..40 {
            let previous = self.0.read(CATALOG).await.map_err(failure)?;
            let mut entries = records(previous.value.clone())?;
            if entries
                .iter()
                .any(|entry| entry.connection.id == record.connection.id)
            {
                return Err(McpError::Exists);
            }
            let active = entries
                .iter()
                .filter(|entry| {
                    entry.connection.revoked_at.is_none()
                        && entry.connection.expires_at > record.connection.created_at
                })
                .count();
            if active >= CONNECTION_LIMIT {
                return Err(McpError::Invalid(
                    "Revoke an unused MCP connection before creating another",
                ));
            }
            if entries.len() >= 10_000 {
                return Err(McpError::Invalid(
                    "This deployment has reached its MCP connection history limit",
                ));
            }
            entries.push(record.clone());
            let value = serde_json::to_value(entries)
                .map_err(|error| StoreError::Unavailable(Box::new(error)))?;
            match self.0.commit(vec![Change { previous, value }]).await {
                Ok(()) => return Ok(()),
                Err(Error::StateConflict) => {
                    tokio::time::sleep(std::time::Duration::from_millis(10 * (attempt + 1))).await
                }
                Err(error) => return Err(failure(error).into()),
            }
        }
        Err(StoreError::Conflict.into())
    }

    async fn mcp_revoke(&self, id: &str, at: DateTime<Utc>) -> Result<(), StoreError> {
        for attempt in 0..40 {
            let previous = self.0.read(CATALOG).await.map_err(failure)?;
            let mut entries = records(previous.value.clone())?;
            let Some(entry) = entries.iter_mut().find(|entry| entry.connection.id == id) else {
                return Ok(());
            };
            if entry.connection.revoked_at.is_some() {
                return Ok(());
            }
            entry.connection.revoked_at = Some(at);
            let value = serde_json::to_value(entries)
                .map_err(|error| StoreError::Unavailable(Box::new(error)))?;
            match self.0.commit(vec![Change { previous, value }]).await {
                Ok(()) => return Ok(()),
                Err(Error::StateConflict) => {
                    tokio::time::sleep(std::time::Duration::from_millis(10 * (attempt + 1))).await
                }
                Err(error) => return Err(failure(error)),
            }
        }
        Err(StoreError::Conflict)
    }

    async fn mcp_record(&self, call: &McpCall) -> Result<(), StoreError> {
        #[derive(Serialize)]
        struct Row<'a> {
            id: &'a str,
            at_ms: i64,
            owner_id: &'a str,
            connection_id: &'a str,
            outcome: lens_contract::mcp::McpOutcome,
            data: String,
        }
        let data = serde_json::to_string(call)
            .map_err(|error| StoreError::Unavailable(Box::new(error)))?;
        let body = serde_json::to_string(&Row {
            id: &call.id,
            at_ms: call.at_ms,
            owner_id: &call.owner_id,
            connection_id: &call.connection_id,
            outcome: call.outcome,
            data,
        })
        .map_err(|error| StoreError::Unavailable(Box::new(error)))?;
        self.0
            .command("INSERT INTO lens_mcp_calls FORMAT JSONEachRow", &[], body)
            .await
            .map_err(failure)?;
        Ok(())
    }

    async fn mcp_activity(&self, query: ActivityQuery) -> Result<McpActivity, StoreError> {
        let parameters = [
            ("all", u8::from(query.owner_id.is_none()).to_string()),
            ("owner", query.owner_id.unwrap_or_default()),
            ("connection", query.connection_id.unwrap_or_default()),
            ("start", query.from_ms.to_string()),
            ("end", query.to_ms.to_string()),
            (
                "before_ms",
                query
                    .before
                    .as_ref()
                    .map_or(i64::MAX, |(ms, _)| *ms)
                    .to_string(),
            ),
            (
                "before_id",
                query
                    .before
                    .as_ref()
                    .map_or_else(String::new, |(_, id)| id.clone()),
            ),
        ];
        let body = self.0.command(
            "SELECT connection_id, argMax(data, tuple(at_ms, id)) AS latest, count() AS calls, \
             countIf(outcome != 'success') AS errors, max(at_ms) AS last_used_ms, \
             maxOrNullIf(at_ms, outcome = 'success') AS last_success_ms \
             FROM lens_mcp_calls WHERE ({all:UInt8} = 1 OR owner_id = {owner:String}) \
             AND at_ms >= {start:Int64} AND at_ms <= {end:Int64} AND JSONExtractString(data, 'method') = 'tools/call' \
             GROUP BY connection_id ORDER BY last_used_ms DESC LIMIT 1001 FORMAT JSONEachRow",
            &parameters, String::new(),
        ).await.map_err(failure)?;
        #[derive(Deserialize)]
        struct Group {
            latest: String,
            calls: u64,
            errors: u64,
            last_used_ms: i64,
            last_success_ms: Option<i64>,
        }
        let groups = decode::<Group>(&body)?;
        let usage_truncated = groups.len() > 1000;
        let usage = groups
            .into_iter()
            .take(1000)
            .map(|group| {
                let call: McpCall = serde_json::from_str(&group.latest)
                    .map_err(|error| StoreError::Unavailable(Box::new(error)))?;
                Ok(McpUsage {
                    connection_id: call.connection_id,
                    connection_name: call.connection_name,
                    owner_id: call.owner_id,
                    owner_name: call.owner_name,
                    client: call.client,
                    calls: group.calls,
                    errors: group.errors,
                    last_used_ms: group.last_used_ms,
                    last_success_ms: group.last_success_ms,
                })
            })
            .collect::<Result<Vec<_>, StoreError>>()?;
        let body = self.0.command(
            "SELECT data FROM lens_mcp_calls WHERE ({all:UInt8} = 1 OR owner_id = {owner:String}) \
             AND ({connection:String} = '' OR connection_id = {connection:String}) \
             AND at_ms >= {start:Int64} AND at_ms <= {end:Int64} \
             AND (at_ms < {before_ms:Int64} OR (at_ms = {before_ms:Int64} AND id < {before_id:String})) \
             ORDER BY at_ms DESC, id DESC LIMIT 101 FORMAT JSONEachRow",
            &parameters, String::new(),
        ).await.map_err(failure)?;
        #[derive(Deserialize)]
        struct Row {
            data: String,
        }
        let mut calls = decode::<Row>(&body)?
            .into_iter()
            .map(|row| {
                serde_json::from_str::<McpCall>(&row.data)
                    .map_err(|error| StoreError::Unavailable(Box::new(error)))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let more = calls.len() > 100;
        calls.truncate(100);
        let next_cursor = more
            .then(|| {
                calls
                    .last()
                    .map(|call| format!("{}:{}", call.at_ms, call.id))
            })
            .flatten();
        Ok(McpActivity {
            usage,
            usage_truncated,
            calls,
            next_cursor,
        })
    }
}
