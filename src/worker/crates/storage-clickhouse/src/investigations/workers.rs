use std::time::Duration;

use chrono::{DateTime, Utc};
use lens_contract::investigations::{Scope, Worker};
use lens_investigations::{RepositoryError, WorkerRepository};
use rand::Rng;

use super::{Investigations, decode, document, failure, key};
use crate::{
    Error,
    state::{Change, Head},
};

const ELIGIBLE: &str = "SELECT w.key AS key, w.revision AS revision, w.digest AS digest
    FROM (SELECT * FROM lens_workers FINAL) AS w
    INNER JOIN lens_state_heads AS h USING (key, revision, digest)
    WHERE w.revoked=0 AND w.id > {after:String}
    AND (w.all_teams=1 OR ({all_teams:Bool}=0 AND w.team_id={team_id:String}
        AND ({team_id:String} != '' OR w.api_key_hash={api_key_hash:String})))
    ORDER BY w.id LIMIT 50 FORMAT JSONEachRow";

fn backoff_ceiling(attempt: u64) -> u64 {
    20 * (attempt + 1).min(8)
}

pub(super) async fn backoff(attempt: u64) {
    let delay = rand::thread_rng().gen_range(0..=backoff_ceiling(attempt));
    tokio::time::sleep(Duration::from_millis(delay)).await;
}

impl Investigations {
    async fn update_worker(
        &self,
        worker_id: &str,
        transform: impl Fn(Worker) -> Worker + Send,
    ) -> Result<Option<Worker>, RepositoryError> {
        let key = key("worker", (worker_id,))?;
        for attempt in 0..40 {
            let previous = self.0.read(&key).await.map_err(failure)?;
            if previous.value.is_null() {
                return Ok(None);
            }
            let updated = transform(decode(previous.value.clone())?);
            let value = document(&updated)?;
            if value == previous.value {
                return Ok(Some(updated));
            }
            match self.0.commit(vec![Change { previous, value }]).await {
                Ok(()) => return Ok(Some(updated)),
                Err(Error::StateConflict) => (),
                Err(error) => return Err(failure(error)),
            }
            backoff(attempt).await;
        }
        Err(RepositoryError::Conflict)
    }
}

impl WorkerRepository for Investigations {
    async fn workers(&self) -> Result<Vec<Worker>, RepositoryError> {
        let mut cursor = String::new();
        let mut workers: Vec<Worker> = Vec::new();
        loop {
            let keys = self
                .0
                .keys("worker/", &cursor, 128)
                .await
                .map_err(failure)?;
            let Some(last) = keys.last() else {
                break;
            };
            let references: Vec<_> = keys.iter().map(String::as_str).collect();
            let records = self.0.read_many(&references).await.map_err(failure)?;
            for record in records {
                if !record.value.is_null() {
                    workers.push(decode(record.value)?);
                }
            }
            cursor = last.clone();
        }
        workers.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(workers)
    }

    async fn eligible_workers(
        &self,
        scope: &Scope,
        after: &str,
    ) -> Result<Vec<Worker>, RepositoryError> {
        let body = self
            .0
            .command(
                ELIGIBLE,
                &[
                    ("after", after.into()),
                    ("all_teams", u8::from(scope.all_teams).to_string()),
                    ("team_id", scope.team_id.clone()),
                    ("api_key_hash", scope.api_key_hash.clone()),
                ],
                String::new(),
            )
            .await
            .map_err(failure)?;
        let heads: Vec<Head> = body
            .lines()
            .map(|line| {
                serde_json::from_str(line)
                    .map_err(|error| RepositoryError::Unavailable(Box::new(error)))
            })
            .collect::<Result<_, _>>()?;
        self.0
            .resolve(&heads)
            .await
            .map_err(failure)?
            .into_iter()
            .map(|record| decode(record.value))
            .collect()
    }

    async fn worker(&self, token_hash: &str) -> Result<Option<Worker>, RepositoryError> {
        let token = self
            .0
            .read(&key("worker-token", (token_hash,))?)
            .await
            .map_err(failure)?;
        if token.value.is_null() {
            return Ok(None);
        }
        let id: String = decode(token.value)?;
        let worker = self
            .0
            .read(&key("worker", (&id,))?)
            .await
            .map_err(failure)?;
        if worker.value.is_null() {
            return Ok(None);
        }
        Ok(Some(decode(worker.value)?))
    }

    async fn save_worker(
        &self,
        worker: &Worker,
        token_hash: Option<&str>,
    ) -> Result<(), RepositoryError> {
        let Some(token_hash) = token_hash else {
            self.update_worker(&worker.id, |_| worker.clone()).await?;
            return Ok(());
        };
        let token_key = key("worker-token", (token_hash,))?;
        let worker_key = key("worker", (&worker.id,))?;
        let records = self
            .0
            .read_many(&[&token_key, &worker_key])
            .await
            .map_err(failure)?;
        if records.iter().any(|record| !record.value.is_null()) {
            return Err(RepositoryError::Conflict);
        }
        let changes = records
            .into_iter()
            .zip([document(&worker.id)?, document(worker)?])
            .map(|(previous, value)| Change { previous, value })
            .collect();
        self.0.commit(changes).await.map_err(failure)
    }

    async fn configure_service_worker(
        &self,
        worker: &Worker,
        token_hash: &str,
    ) -> Result<Worker, RepositoryError> {
        for attempt in 0..40 {
            let token = self
                .0
                .read(&key("worker-token", (token_hash,))?)
                .await
                .map_err(failure)?;
            let worker_id = if token.value.is_null() {
                worker.id.clone()
            } else {
                decode(token.value.clone())?
            };
            let previous = self
                .0
                .read(&key("worker", (&worker_id,))?)
                .await
                .map_err(failure)?;
            if token.value.is_null() && !previous.value.is_null() {
                return Err(RepositoryError::Conflict);
            }
            let updated = Worker {
                id: worker_id,
                ..worker.clone()
            };
            let mut changes = vec![Change {
                previous,
                value: document(&updated)?,
            }];
            if token.value.is_null() {
                changes.push(Change {
                    previous: token,
                    value: document(&updated.id)?,
                });
            }
            match self.0.commit(changes).await {
                Ok(()) => return Ok(updated),
                Err(Error::StateConflict) => (),
                Err(error) => return Err(failure(error)),
            }
            backoff(attempt).await;
        }
        Err(RepositoryError::Conflict)
    }

    async fn set_worker_billing(
        &self,
        worker_id: &str,
        key_id: &str,
    ) -> Result<Option<Worker>, RepositoryError> {
        Ok(self
            .update_worker(worker_id, |worker| {
                if worker.revoked {
                    worker
                } else {
                    Worker {
                        analysis_key_id: Some(key_id.into()),
                        ..worker
                    }
                }
            })
            .await?
            .filter(|worker| !worker.revoked))
    }

    async fn revoke_worker(&self, worker_id: &str) -> Result<(), RepositoryError> {
        self.update_worker(worker_id, |worker| Worker {
            revoked: true,
            ..worker
        })
        .await?;
        Ok(())
    }

    async fn heartbeat(&self, worker_id: &str, now: DateTime<Utc>) -> Result<(), RepositoryError> {
        self.update_worker(worker_id, |worker| Worker {
            last_seen: now,
            ..worker
        })
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::backoff_ceiling;
    use rstest::rstest;

    #[rstest]
    #[case::first(0, 20)]
    #[case::second(1, 40)]
    #[case::cap(7, 160)]
    #[case::capped(8, 160)]
    #[case::last(39, 160)]
    fn worker_retries_have_bounded_backoff(#[case] attempt: u64, #[case] ceiling: u64) {
        assert_eq!(backoff_ceiling(attempt), ceiling);
    }
}
