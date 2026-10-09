mod history;
mod reviews;
mod scheduling;
mod schema;
mod trace_findings;
mod workers;
mod writes;

use chrono::{DateTime, Utc};
use lens_contract::{
    investigations::{Lens, Scope, public},
    worker::Job,
};
use lens_investigations::{LensRepository, RepositoryError, can_access, due_at};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{
    Error,
    state::{Change, ClickHouseState, Snapshot},
};

const KEY_ENCODING: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

#[derive(Clone)]
pub struct Investigations(pub ClickHouseState);

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredLens {
    lens: Lens,
    #[serde(deserialize_with = "Option::deserialize")]
    due_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ArchivedJob {
    lens_id: String,
    parent_key: String,
    archived_version: i64,
    #[serde(serialize_with = "public::serialize")]
    job: Job,
}

fn failure(error: Error) -> RepositoryError {
    match error {
        Error::StateConflict | Error::StateExists => RepositoryError::Conflict,
        error => RepositoryError::Unavailable(Box::new(error)),
    }
}

fn key(namespace: &str, identity: impl Serialize) -> Result<String, RepositoryError> {
    let identity = serde_json::to_string(&identity)
        .map_err(|error| RepositoryError::Unavailable(Box::new(error)))?;
    Ok(format!(
        "{namespace}/{}",
        utf8_percent_encode(&identity, KEY_ENCODING)
    ))
}

fn document(value: impl Serialize) -> Result<Value, RepositoryError> {
    serde_json::to_value(value).map_err(|error| RepositoryError::Unavailable(Box::new(error)))
}

fn decode<T: DeserializeOwned>(value: Value) -> Result<T, RepositoryError> {
    if value.is_array() {
        return Err(failure(Error::InvalidState));
    }
    serde_json::from_value(value).map_err(|error| RepositoryError::Unavailable(Box::new(error)))
}

fn stored(lens: &Lens) -> Result<Value, RepositoryError> {
    document(StoredLens {
        lens: lens.clone(),
        due_at: due_at(lens),
    })
}

impl Investigations {
    async fn snapshot(&self, id: &str) -> Result<Snapshot, RepositoryError> {
        self.0.read(&key("lens", (id,))?).await.map_err(failure)
    }

    async fn archive_changes(
        &self,
        parent: &Snapshot,
        previous: &Lens,
        updated: &Lens,
    ) -> Result<Vec<Change>, RepositoryError> {
        let removed: Vec<_> = previous
            .jobs
            .iter()
            .filter(|job| !updated.jobs.iter().any(|retained| retained.id == job.id))
            .collect();
        let keys: Vec<_> = removed
            .iter()
            .map(|job| key("run", (&previous.id, &job.id)))
            .collect::<Result<_, _>>()?;
        let references: Vec<_> = keys.iter().map(String::as_str).collect();
        let snapshots = self.0.read_many(&references).await.map_err(failure)?;
        removed
            .into_iter()
            .zip(snapshots)
            .filter(|(_, snapshot)| snapshot.value.is_null())
            .map(|(job, snapshot)| {
                Ok(Change {
                    previous: snapshot,
                    value: document(ArchivedJob {
                        lens_id: previous.id.clone(),
                        parent_key: parent.head.key.clone(),
                        archived_version: updated.version,
                        job: job.clone(),
                    })?,
                })
            })
            .collect()
    }

    async fn publish_lens(
        &self,
        writer: writes::Writer,
        previous: Snapshot,
        current: &Lens,
        candidate: &Lens,
        checkpoint: Option<Change>,
    ) -> Result<Lens, RepositoryError> {
        if current.id != candidate.id {
            return Err(failure(Error::InvalidState));
        }
        let changed = document(candidate)? != document(current)?;
        if !changed && checkpoint.is_none() {
            writer.commit(Vec::new()).await?;
            return Ok(current.clone());
        }
        let updated = Lens {
            version: if changed {
                current
                    .version
                    .checked_add(1)
                    .ok_or_else(|| failure(Error::InvalidState))?
            } else {
                current.version
            },
            ..candidate.clone()
        };
        let archives = self.archive_changes(&previous, current, &updated).await?;
        let changes = std::iter::once(Change {
            previous,
            value: stored(&updated)?,
        })
        .chain(archives)
        .chain(checkpoint)
        .collect();
        writer.commit(changes).await?;
        Ok(updated)
    }
}

impl LensRepository for Investigations {
    async fn lenses(&self, scope: &Scope) -> Result<Vec<Lens>, RepositoryError> {
        let mut cursor = String::new();
        let mut lenses = Vec::new();
        loop {
            let keys = self.0.keys("lens/", &cursor, 128).await.map_err(failure)?;
            let Some(last) = keys.last() else {
                break;
            };
            let references: Vec<_> = keys.iter().map(String::as_str).collect();
            let records = self.0.read_many(&references).await.map_err(failure)?;
            for record in records {
                if record.value.is_null() {
                    continue;
                }
                let stored: StoredLens = decode(record.value)?;
                if can_access(scope, &stored.lens.scope) {
                    lenses.push(stored.lens);
                }
            }
            cursor = last.clone();
        }
        lenses.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(lenses)
    }

    async fn get(&self, lens_id: &str) -> Result<Option<Lens>, RepositoryError> {
        let snapshot = self.snapshot(lens_id).await?;
        if snapshot.value.is_null() {
            return Ok(None);
        }
        Ok(Some(decode::<StoredLens>(snapshot.value)?.lens))
    }

    async fn create(&self, lens: &Lens) -> Result<Lens, RepositoryError> {
        let writer = self.writer(&lens.id).await?;
        let previous = self.snapshot(&lens.id).await?;
        if !previous.value.is_null() {
            return Err(RepositoryError::Conflict);
        }
        writer
            .commit(vec![Change {
                previous,
                value: stored(lens)?,
            }])
            .await?;
        Ok(lens.clone())
    }

    async fn replace(&self, expected: &Lens, candidate: &Lens) -> Result<Lens, RepositoryError> {
        if expected.id != candidate.id {
            return Err(failure(Error::InvalidState));
        }
        let writer = self.writer(&expected.id).await?;
        let previous = self.snapshot(&expected.id).await?;
        if previous.value.is_null() {
            return Err(RepositoryError::Conflict);
        }
        let current = decode::<StoredLens>(previous.value.clone())?.lens;
        if current.version != expected.version {
            return Err(RepositoryError::Conflict);
        }
        self.publish_lens(writer, previous, &current, candidate, None)
            .await
    }

    async fn jobs(&self, lens_id: &str, offset: u64) -> Result<Vec<Job>, RepositoryError> {
        self.history(lens_id, offset, None).await
    }

    async fn job(&self, lens_id: &str, job_id: &str) -> Result<Option<Job>, RepositoryError> {
        Ok(self
            .history(lens_id, 0, Some(job_id))
            .await?
            .into_iter()
            .next())
    }
}
