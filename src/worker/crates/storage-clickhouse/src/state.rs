mod maintenance;
mod records;
mod retry;
mod transport;

use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use litellm_http::Client;
use serde::Deserialize;
use serde_json::Value;

use crate::{Connection, Error};
use records::Blob;
pub use records::{Change, Head, PreparedCommit, Snapshot};
pub(crate) use retry::backoff;
use transport::{command, decode, encode};

#[derive(Clone)]
pub struct ClickHouseState {
    client: Client,
    connection: Connection,
}

impl ClickHouseState {
    pub fn new(client: Client, connection: Connection) -> Self {
        Self { client, connection }
    }

    pub async fn initialize(&self, keeper_path: &str) -> Result<(), Error> {
        self.command(
            "CREATE TABLE IF NOT EXISTS lens_state_heads \
             (key String, revision UInt64, digest String) \
             ENGINE=KeeperMap({keeper_path:String}) PRIMARY KEY key",
            &[("keeper_path", keeper_path.to_owned())],
            String::new(),
        )
        .await?;
        self.command(
            "CREATE TABLE IF NOT EXISTS lens_state_blobs \
             (key String, revision UInt64, digest FixedString(64), data String CODEC(ZSTD(3)), \
             created_at DateTime64(3) DEFAULT now64(3)) \
             ENGINE=ReplacingMergeTree ORDER BY (key, revision, digest) \
             SETTINGS fsync_after_insert=1, fsync_part_directory=1",
            &[],
            String::new(),
        )
        .await?;
        Ok(())
    }

    pub(crate) async fn command(
        &self,
        query: &'static str,
        parameters: &[(&str, String)],
        body: String,
    ) -> Result<String, Error> {
        command(&self.client, &self.connection, query, parameters, body).await
    }

    async fn ensure_head(&self, key: &str) -> Result<(), Error> {
        match self
            .command(
                "INSERT INTO lens_state_heads FORMAT JSONEachRow",
                &[],
                encode(&Head::empty(key))?,
            )
            .await
        {
            Ok(_) | Err(Error::StateExists) => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub async fn heads(&self, keys: &[&str]) -> Result<Vec<Head>, Error> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let body = self.command(
            "SELECT key, revision, digest FROM lens_state_heads \
             WHERE key IN JSONExtract({keys:String}, 'Array(String)') ORDER BY key FORMAT JSONEachRow",
            &[("keys", encode(&keys)?)], String::new(),
        ).await?;
        let mut by_key = BTreeMap::new();
        for head in decode::<Head>(&body)? {
            if !head.valid()
                || !keys.contains(&head.key.as_str())
                || by_key.insert(head.key.clone(), head).is_some()
            {
                return Err(Error::InvalidResponse);
            }
        }
        Ok(keys
            .iter()
            .map(|key| {
                by_key
                    .get(*key)
                    .cloned()
                    .unwrap_or_else(|| Head::empty(*key))
            })
            .collect())
    }

    pub async fn read(&self, key: &str) -> Result<Snapshot, Error> {
        self.read_many(&[key])
            .await?
            .into_iter()
            .next()
            .ok_or(Error::InvalidResponse)
    }

    pub async fn keys(&self, prefix: &str, after: &str, limit: u32) -> Result<Vec<String>, Error> {
        let body = self.command(
            "SELECT DISTINCT key FROM lens_state_blobs \
             WHERE key >= {prefix:String} AND key < concat({prefix:String}, char(127)) AND key > {after:String} \
             ORDER BY key LIMIT {limit:UInt32} FORMAT JSONEachRow",
            &[("prefix", prefix.to_owned()), ("after", after.to_owned()), ("limit", limit.to_string())], String::new(),
        ).await?;
        #[derive(Deserialize)]
        struct Key {
            key: String,
        }
        Ok(decode::<Key>(&body)?
            .into_iter()
            .map(|row| row.key)
            .collect())
    }

    pub async fn read_many(&self, keys: &[&str]) -> Result<Vec<Snapshot>, Error> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        for attempt in 0..8 {
            let before = self.heads(keys).await?;
            let values = self.values(&before).await?;
            if (keys.len() == 1 || before == self.heads(keys).await?)
                && let Some(values) = values
            {
                return Ok(values);
            }
            tokio::time::sleep(Duration::from_millis(2 * (attempt + 1))).await;
        }
        Err(Error::StateUnavailable)
    }

    async fn values(&self, heads: &[Head]) -> Result<Option<Vec<Snapshot>>, Error> {
        if heads.is_empty() {
            return Ok(Some(Vec::new()));
        }
        if heads.iter().any(|head| !head.valid()) {
            return Err(Error::InvalidState);
        }
        let references: Vec<_> = heads
            .iter()
            .filter(|head| !head.digest.is_empty())
            .map(|head| (&head.key, head.revision, &head.digest))
            .collect();
        let body = self.command(
            "SELECT key, revision, digest, data FROM lens_state_blobs FINAL \
             WHERE (key, revision, digest) IN \
             JSONExtract({references:String}, 'Array(Tuple(String, UInt64, String))') FORMAT JSONEachRow",
            &[("references", encode(&references)?)], String::new(),
        ).await?;
        let mut values = BTreeMap::new();
        for blob in decode::<Blob>(&body)? {
            if !heads.contains(&blob.head) {
                return Err(Error::InvalidResponse);
            }
            let snapshot = blob.snapshot()?;
            if values.insert(snapshot.head.clone(), snapshot).is_some() {
                return Err(Error::InvalidResponse);
            }
        }
        Ok(heads
            .iter()
            .map(|head| {
                if head.digest.is_empty() {
                    Some(Snapshot {
                        head: head.clone(),
                        value: Value::Null,
                    })
                } else {
                    values.get(head).cloned()
                }
            })
            .collect())
    }

    pub async fn resolve(&self, heads: &[Head]) -> Result<Vec<Snapshot>, Error> {
        if let Some(values) = self.values(heads).await? {
            return Ok(values);
        }
        let keys: Vec<_> = heads.iter().map(|head| head.key.as_str()).collect();
        let current = self.read_many(&keys).await?;
        if heads
            .iter()
            .zip(&current)
            .all(|(old, new)| old.digest == new.head.digest)
        {
            Ok(current)
        } else {
            Err(Error::StateUnavailable)
        }
    }

    pub async fn prepare(&self, changes: Vec<Change>) -> Result<PreparedCommit, Error> {
        let keys: BTreeSet<_> = changes
            .iter()
            .map(|change| &change.previous.head.key)
            .collect();
        if changes.is_empty()
            || keys.len() != changes.len()
            || changes.iter().any(|change| !change.previous.head.valid())
        {
            return Err(Error::InvalidState);
        }
        let blobs = changes
            .iter()
            .map(Blob::next)
            .collect::<Result<Vec<_>, _>>()?;
        let body = blobs
            .iter()
            .map(encode)
            .collect::<Result<Vec<_>, _>>()?
            .join("\n");
        for key in keys {
            self.ensure_head(key).await?;
        }
        self.command("INSERT INTO lens_state_blobs FORMAT JSONEachRow", &[], body)
            .await?;
        Ok(PreparedCommit { changes, blobs })
    }

    pub async fn publish(&self, prepared: &PreparedCommit) -> Result<(), Error> {
        let keys: Vec<_> = prepared.blobs.iter().map(|blob| &blob.head.key).collect();
        let versions: BTreeMap<_, _> = prepared
            .changes
            .iter()
            .map(|change| (&change.previous.head.key, change.previous.head.revision))
            .collect();
        let digests: BTreeMap<_, _> = prepared
            .blobs
            .iter()
            .map(|blob| (&blob.head.key, &blob.head.digest))
            .collect();
        self.command(
            "ALTER TABLE lens_state_heads UPDATE \
             revision=revision+1+throwIf(revision != \
             JSONExtract({versions:String}, 'Map(String, UInt64)')[key], 'LENS_STATE_CONFLICT'), \
             digest=JSONExtract({digests:String}, 'Map(String, String)')[key] \
             WHERE key IN JSONExtract({keys:String}, 'Array(String)')",
            &[
                ("keys", encode(&keys)?),
                ("versions", encode(&versions)?),
                ("digests", encode(&digests)?),
            ],
            String::new(),
        )
        .await?;
        Ok(())
    }

    pub async fn commit(&self, changes: Vec<Change>) -> Result<(), Error> {
        self.publish(&self.prepare(changes).await?).await
    }

    pub async fn update(
        &self,
        key: &str,
        transform: impl Fn(&Value) -> Value,
        attempts: u32,
    ) -> Result<Snapshot, Error> {
        for attempt in 0..attempts {
            let previous = self.read(key).await?;
            let value = transform(&previous.value);
            if value == previous.value {
                return Ok(previous);
            }
            let prepared = self.prepare(vec![Change { previous, value }]).await?;
            match self.publish(&prepared).await {
                Ok(()) => {
                    return prepared
                        .blobs
                        .into_iter()
                        .next()
                        .ok_or(Error::InvalidState)?
                        .snapshot();
                }
                Err(Error::StateConflict) => (),
                Err(error) => return Err(error),
            }
            backoff(u64::from(attempt)).await;
        }
        Err(Error::StateConflict)
    }
}
