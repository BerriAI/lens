use std::time::Duration;

use chrono::{DateTime, Utc};
use lens_contract::datasets::{Dataset, DatasetSummary};
use lens_datasets::{
    DatasetRepository, Evidence, Finding, ReadError, Scope, StoreError, StoredSummary, can_access,
};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use rand::Rng;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{
    Error,
    state::{Change, ClickHouseState},
};

const KEY_ENCODING: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

#[derive(Clone)]
pub struct Datasets(pub ClickHouseState);

#[derive(Clone)]
pub struct Findings(pub ClickHouseState);

#[derive(Deserialize)]
struct StoredLens {
    lens: LensFindings,
}

#[derive(Deserialize)]
struct LensFindings {
    scope: Scope,
    #[serde(default)]
    findings: Vec<StoredFinding>,
}

#[derive(Deserialize)]
struct StoredFinding {
    id: String,
    evidence: Vec<StoredEvidence>,
}

#[derive(Deserialize)]
struct StoredEvidence {
    execution_id: String,
    span_id: String,
}

impl Findings {
    pub async fn get(
        &self,
        lens_id: &str,
        ids: &[String],
        scope: &Scope,
    ) -> Result<Vec<Finding>, ReadError> {
        let key = key("lens", (lens_id,))?;
        let snapshot = self.0.read(&key).await.map_err(failure)?;
        if snapshot.value.is_null() {
            return Err(ReadError::LensNotFound);
        }
        let stored: StoredLens = decode(snapshot.value)?;
        if !can_access(scope, &stored.lens.scope) {
            return Err(ReadError::LensNotFound);
        }
        Ok(stored
            .lens
            .findings
            .into_iter()
            .filter(|finding| ids.contains(&finding.id))
            .map(|finding| Finding {
                id: finding.id,
                evidence: finding
                    .evidence
                    .into_iter()
                    .map(|evidence| Evidence {
                        execution_id: evidence.execution_id,
                        span_id: evidence.span_id,
                    })
                    .collect(),
            })
            .collect())
    }
}

fn failure(error: Error) -> StoreError {
    match error {
        Error::StateConflict | Error::StateExists => StoreError::Conflict,
        error => StoreError::Unavailable(Box::new(error)),
    }
}

fn key(namespace: &str, identity: impl Serialize) -> Result<String, StoreError> {
    let identity = serde_json::to_string(&identity)
        .map_err(|error| StoreError::Unavailable(Box::new(error)))?;
    Ok(format!(
        "{namespace}/{}",
        utf8_percent_encode(&identity, KEY_ENCODING)
    ))
}

fn document(value: impl Serialize) -> Result<Value, StoreError> {
    serde_json::to_value(value).map_err(|error| StoreError::Unavailable(Box::new(error)))
}

fn decode<T: DeserializeOwned>(value: Value) -> Result<T, StoreError> {
    serde_json::from_value(value).map_err(|error| StoreError::Unavailable(Box::new(error)))
}

fn backoff_ceiling(attempt: u64) -> u64 {
    20 * (attempt + 1).min(8)
}

impl DatasetRepository for Datasets {
    async fn get(&self, id: &str, revision: Option<i64>) -> Result<Option<Dataset>, StoreError> {
        let revision = match revision {
            Some(revision) => revision,
            None => {
                let latest = self
                    .0
                    .read(&key("dataset-latest", (id,))?)
                    .await
                    .map_err(failure)?;
                if latest.value.is_null() {
                    return Ok(None);
                }
                decode::<StoredSummary>(latest.value)?.summary.revision
            }
        };
        let record = self
            .0
            .read(&key("dataset", (id, revision))?)
            .await
            .map_err(failure)?;
        if record.value.is_null() {
            return Ok(None);
        }
        decode(record.value).map(Some)
    }

    async fn summaries(&self) -> Result<Vec<StoredSummary>, StoreError> {
        let mut cursor = String::new();
        let mut summaries = Vec::new();
        loop {
            let keys = self
                .0
                .keys("dataset-latest/", &cursor, 128)
                .await
                .map_err(failure)?;
            let Some(last) = keys.last() else {
                break;
            };
            let references: Vec<_> = keys.iter().map(String::as_str).collect();
            let records = self.0.read_many(&references).await.map_err(failure)?;
            for record in records {
                if !record.value.is_null() {
                    summaries.push(decode::<StoredSummary>(record.value)?);
                }
            }
            cursor = last.clone();
        }
        summaries.sort_by_key(|entry| std::cmp::Reverse(entry.summary.updated_at));
        Ok(summaries)
    }

    async fn insert(&self, dataset: &Dataset, saved_at: DateTime<Utc>) -> Result<bool, StoreError> {
        let summary = StoredSummary {
            team_id: dataset.team_id.clone(),
            summary: DatasetSummary {
                id: dataset.id.clone(),
                name: dataset.name.clone(),
                agent_name: dataset.agent_name.clone(),
                revision: dataset.revision,
                case_count: dataset.cases.len(),
                updated_at: saved_at,
            },
        };
        let revision_key = key("dataset", (&dataset.id, dataset.revision))?;
        let latest_key = key("dataset-latest", (&dataset.id,))?;
        for attempt in 0..40 {
            let [revision, latest] = self
                .0
                .read_many(&[&revision_key, &latest_key])
                .await
                .map_err(failure)?
                .try_into()
                .map_err(|_| failure(Error::InvalidResponse))?;
            if !revision.value.is_null() {
                return Ok(false);
            }
            let advance = latest.value.is_null()
                || dataset.revision
                    > decode::<StoredSummary>(latest.value.clone())?
                        .summary
                        .revision;
            let revision = Change {
                previous: revision,
                value: document(dataset)?,
            };
            let changes = if advance {
                vec![
                    revision,
                    Change {
                        previous: latest,
                        value: document(&summary)?,
                    },
                ]
            } else {
                vec![revision]
            };
            match self.0.commit(changes).await {
                Ok(()) => return Ok(true),
                Err(Error::StateConflict) => (),
                Err(error) => return Err(failure(error)),
            }
            let delay = rand::thread_rng().gen_range(0..=backoff_ceiling(attempt));
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }
        Err(StoreError::Conflict)
    }
}

#[cfg(test)]
mod tests {
    use super::backoff_ceiling;
    use rstest::rstest;

    #[rstest]
    #[case::first_retry(0, 20)]
    #[case::second_retry(1, 40)]
    #[case::cap(7, 160)]
    #[case::capped(8, 160)]
    #[case::last_retry(39, 160)]
    fn retries_have_bounded_backoff(#[case] attempt: u64, #[case] ceiling_ms: u64) {
        assert_eq!(backoff_ceiling(attempt), ceiling_ms);
    }
}
