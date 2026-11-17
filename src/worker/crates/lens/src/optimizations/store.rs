use chrono::Utc;
use lens_contract::optimizations::{
    OptimizationConfig, OptimizationResult, OptimizationRun, OptimizationStatus, ReplayRequest,
};
use litellm_storage_clickhouse::state::{Change, ClickHouseState, Snapshot};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::OptimizationError as Error;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Record {
    pub id: String,
    pub config: OptimizationConfig,
    pub status: OptimizationStatus,
    pub created_at: i64,
    pub error: Option<String>,
    pub scan_cursor: Option<String>,
    pub scan_until: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Attempt {
    pub result: OptimizationResult,
    pub request: Option<ReplayRequest>,
    pub context: String,
    pub trace_ref: String,
    pub state: AttemptState,
    pub owner: Option<String>,
    pub deadline: i64,
    pub external: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum AttemptState {
    Pending,
    Running,
    Complete,
}

#[derive(Clone)]
pub(super) struct Store(pub ClickHouseState);

pub(super) fn digest(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

pub(super) fn key(id: &str) -> Result<String, Error> {
    uuid::Uuid::parse_str(id).map_err(|_| Error::NotFound)?;
    Ok(format!("optimization/{id}"))
}

pub(super) fn attempt_prefix(id: &str) -> Result<String, Error> {
    key(id)?;
    Ok(format!("optimization-attempt/{id}/"))
}

impl Store {
    pub async fn get(&self, id: &str) -> Result<(Snapshot, Record), Error> {
        let snapshot = self.0.read(&key(id)?).await?;
        if snapshot.value.is_null() {
            return Err(Error::NotFound);
        }
        let record: Record = serde_json::from_value(snapshot.value.clone())?;
        Ok((snapshot, record))
    }

    pub async fn records(&self) -> Result<Vec<Record>, Error> {
        let keys = self.0.keys("optimization/", "", 100).await?;
        let refs: Vec<_> = keys.iter().map(String::as_str).collect();
        self.0
            .read_many(&refs)
            .await?
            .into_iter()
            .filter(|snapshot| !snapshot.value.is_null())
            .map(|snapshot| serde_json::from_value(snapshot.value).map_err(Error::from))
            .collect()
    }

    pub async fn attempts(&self, id: &str) -> Result<Vec<(Snapshot, Attempt)>, Error> {
        let keys = self.0.keys(&attempt_prefix(id)?, "", 5000).await?;
        let refs: Vec<_> = keys.iter().map(String::as_str).collect();
        self.0
            .read_many(&refs)
            .await?
            .into_iter()
            .filter(|snapshot| !snapshot.value.is_null())
            .map(|snapshot| Ok((snapshot.clone(), serde_json::from_value(snapshot.value)?)))
            .collect()
    }

    pub async fn view(&self, record: Record) -> Result<OptimizationRun, Error> {
        let attempts = self.attempts(&record.id).await?;
        let pending = attempts
            .iter()
            .filter(|(_, attempt)| attempt.state != AttemptState::Complete)
            .count();
        Ok(OptimizationRun {
            id: record.id,
            config: record.config,
            status: record.status,
            created_at: record.created_at,
            error: record.error,
            pending,
            results: attempts
                .into_iter()
                .filter(|(_, attempt)| attempt.state == AttemptState::Complete)
                .map(|(_, attempt)| attempt.result)
                .collect(),
        })
    }

    pub async fn insert_attempt(&self, id: &str, attempt: &Attempt) -> Result<(), Error> {
        let previous = self
            .0
            .read(&format!("{}{}", attempt_prefix(id)?, attempt.result.id))
            .await?;
        if !previous.value.is_null() {
            return Ok(());
        }
        let (record_snapshot, record) = self.get(id).await?;
        if record.status != OptimizationStatus::Running {
            return Ok(());
        }
        match self
            .0
            .commit(vec![
                Change {
                    previous: record_snapshot,
                    value: serde_json::to_value(record)?,
                },
                Change {
                    previous,
                    value: serde_json::to_value(attempt)?,
                },
            ])
            .await
            .map_err(Error::from)
        {
            Err(Error::Storage(litellm_storage_clickhouse::Error::StateConflict)) => Ok(()),
            result => result,
        }
    }

    pub async fn save(&self, previous: Snapshot, value: &impl Serialize) -> Result<(), Error> {
        self.0
            .commit(vec![Change {
                previous,
                value: serde_json::to_value(value)?,
            }])
            .await?;
        Ok(())
    }

    pub async fn claim(
        &self,
        id: &str,
        snapshot: Snapshot,
        mut attempt: Attempt,
    ) -> Result<Option<Attempt>, Error> {
        if attempt.state != AttemptState::Pending {
            return Ok(None);
        }
        let (record_snapshot, record) = self.get(id).await?;
        if record.status != OptimizationStatus::Running {
            return Ok(None);
        }
        attempt.state = AttemptState::Running;
        attempt.owner = Some(uuid::Uuid::new_v4().to_string());
        attempt.deadline = Utc::now().timestamp_millis() + 600_000;
        match self
            .0
            .commit(vec![
                Change {
                    previous: record_snapshot,
                    value: serde_json::to_value(record)?,
                },
                Change {
                    previous: snapshot,
                    value: serde_json::to_value(&attempt)?,
                },
            ])
            .await
            .map_err(Error::from)
        {
            Ok(()) => Ok(Some(attempt)),
            Err(Error::Storage(litellm_storage_clickhouse::Error::StateConflict)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub async fn finish(&self, run: &str, mut attempt: Attempt) -> Result<(), Error> {
        let snapshot = self
            .0
            .read(&format!("{}{}", attempt_prefix(run)?, attempt.result.id))
            .await?;
        let current: Attempt = serde_json::from_value(snapshot.value.clone())?;
        if current.owner != attempt.owner || current.state != AttemptState::Running {
            return Err(Error::Conflict);
        }
        attempt.state = AttemptState::Complete;
        attempt.result.completed_at = Utc::now().timestamp_millis();
        self.save(snapshot, &attempt).await
    }
}
