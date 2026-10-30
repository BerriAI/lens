use std::{collections::BTreeMap, time::Instant};

use chrono::{DateTime, Utc};
use lens_contract::{
    investigations::{Lens, Public},
    worker::{Job, JobStatus, Progress, Review, ReviewVersion},
};
use lens_investigations::{
    CheckpointError, RepositoryError, apply_progress, criteria_key, current_job, replace_job,
};
use tokio::time::{Instant as Deadline, timeout_at};

use super::{
    Investigations, StoredLens, decode, document, failure, key,
    pending::{Queued, Write},
    workers::backoff,
    writes::{WRITE_RETRY_WINDOW, Writer, writer_key},
};
use crate::{
    Error,
    state::{Change, Snapshot},
};

fn review_key(lens_id: &str, job: &Job, execution_id: &str) -> Result<String, RepositoryError> {
    let criteria = criteria_key(&job.settings)
        .map_err(|error| RepositoryError::Unavailable(Box::new(error)))?;
    key("review", (lens_id, criteria, execution_id))
}

fn progress_time(now: DateTime<Utc>, started: Instant) -> Result<DateTime<Utc>, CheckpointError> {
    let elapsed = chrono::Duration::from_std(started.elapsed())
        .map_err(|_| lens_investigations::Error::CalendarRange("Job progress"))?;
    now.checked_add_signed(elapsed)
        .ok_or_else(|| lens_investigations::Error::CalendarRange("Job progress").into())
}

fn live_lease(lease: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    lease.is_some_and(|lease| lease > now)
}

fn progressed(lens: &Lens, write: &Write) -> Result<Job, CheckpointError> {
    let now = progress_time(write.now, write.started)?;
    let job = current_job(lens).ok_or(CheckpointError::Ownership)?;
    if job.id != write.assigned.id
        || job.worker_id != write.assigned.worker_id
        || job.attempts != write.assigned.attempts
        || job.status != JobStatus::Running
        || !live_lease(job.lease_until, now)
    {
        return Err(CheckpointError::Ownership);
    }
    Ok(apply_progress(job, &write.body, now)?)
}

fn combine(lens: &Lens, claimed: Vec<Queued>) -> (Lens, Vec<Queued>) {
    let mut candidate = lens.clone();
    let mut included = Vec::new();
    for queued in claimed {
        match progressed(&candidate, &queued.write) {
            Ok(job) => {
                candidate = replace_job(&candidate, job);
                included.push(queued);
            }
            Err(error) => queued.settle(Err(error)),
        }
    }
    (candidate, included)
}

fn retry_after(claimed: Vec<Queued>, error: RepositoryError) -> Vec<Queued> {
    match error {
        RepositoryError::Conflict => claimed
            .into_iter()
            .map(|queued| queued.retried(false))
            .collect(),
        RepositoryError::WriteUnconfirmed => claimed
            .into_iter()
            .filter_map(|queued| {
                if queued.write.body.review.is_some() {
                    queued.settle(Err(RepositoryError::WriteUnconfirmed.into()));
                    return None;
                }
                Some(queued.retried(true))
            })
            .collect(),
        error => {
            let mut claimed = claimed.into_iter();
            if let Some(first) = claimed.next() {
                first.settle(Err(error.into()));
            }
            claimed.collect()
        }
    }
}

impl Investigations {
    pub async fn reviews(&self, lens_id: &str, job: &Job) -> Result<Vec<Review>, RepositoryError> {
        let keys = job
            .sample
            .iter()
            .flat_map(|sample| &sample.executions)
            .map(|execution| review_key(lens_id, job, &execution.id))
            .collect::<Result<Vec<_>, _>>()?;
        let references: Vec<_> = keys.iter().map(String::as_str).collect();
        self.0
            .read_many(&references)
            .await
            .map_err(failure)?
            .into_iter()
            .filter(|record| !record.value.is_null())
            .map(|record| decode(record.value))
            .collect()
    }

    pub async fn progress(
        &self,
        lens_id: &str,
        assigned: &Job,
        body: &Progress,
        now: DateTime<Utc>,
    ) -> Result<Option<Lens>, CheckpointError> {
        let started = Instant::now();
        let checkpoint = body.review.as_ref().filter(|review| {
            !review.reused && review.extraction.is_some() && !review.content_version.is_empty()
        });
        let cache = checkpoint
            .map(|review| {
                Ok::<_, RepositoryError>((
                    review_key(lens_id, assigned, &review.execution_id)?,
                    document(Public(review))?,
                ))
            })
            .transpose()?;
        let turn_key = writer_key(lens_id)?;
        let deadline = Deadline::now() + WRITE_RETRY_WINDOW;
        let pending = self.0.pending();
        let (ticket, mut saved) = pending.queue(
            lens_id,
            Write {
                assigned: assigned.clone(),
                body: body.clone(),
                now,
                started,
                deadline,
                checkpoint: cache,
            },
        );
        tokio::spawn(self.clone().publish_progress(lens_id.to_owned(), turn_key));
        if let Ok(result) = timeout_at(deadline, &mut saved).await {
            return result.unwrap_or_else(|_| Err(RepositoryError::WriteUnconfirmed.into()));
        }
        if let Some(queued) = pending.withdraw(lens_id, ticket) {
            return Err(queued.failure().into());
        }
        saved
            .await
            .unwrap_or_else(|_| Err(RepositoryError::WriteUnconfirmed.into()))
    }

    async fn publish_progress(self, lens_id: String, turn_key: String) {
        let pending = self.0.pending();
        for attempt in 0.. {
            let turn = self.0.turn(&turn_key).await;
            let claimed = pending.claim(&lens_id);
            if claimed.is_empty() {
                return;
            }
            let retry = self.publish_claimed(&lens_id, claimed).await;
            if retry.is_empty() {
                return;
            }
            pending.restore(&lens_id, retry);
            drop(turn);
            backoff(attempt).await;
        }
    }

    async fn publish_claimed(&self, lens_id: &str, claimed: Vec<Queued>) -> Vec<Queued> {
        let (writer, previous, current) = match self.open(lens_id).await {
            Ok(Some(opened)) => opened,
            Ok(None) => {
                claimed
                    .into_iter()
                    .for_each(|queued| queued.settle(Ok(None)));
                return Vec::new();
            }
            Err(error) => return retry_after(claimed, error),
        };
        let (candidate, included) = combine(&current.lens, claimed);
        let checkpoints = match self.checkpoints(&included).await {
            Ok(checkpoints) => checkpoints,
            Err(error) => return retry_after(included, error),
        };
        match self
            .publish_lens(writer, previous, &current.lens, &candidate, checkpoints)
            .await
        {
            Ok(lens) => {
                included
                    .into_iter()
                    .for_each(|queued| queued.settle(Ok(Some(lens.clone()))));
                Vec::new()
            }
            Err(error) => retry_after(included, error),
        }
    }

    async fn open(
        &self,
        lens_id: &str,
    ) -> Result<Option<(Writer, Snapshot, StoredLens)>, RepositoryError> {
        let writer = self.lease(lens_id).await?;
        let previous = self.snapshot(lens_id).await?;
        if previous.value.is_null() {
            return Ok(None);
        }
        let current = decode(previous.value.clone())?;
        Ok(Some((writer, previous, current)))
    }

    async fn checkpoints(&self, included: &[Queued]) -> Result<Vec<Change>, RepositoryError> {
        let values: BTreeMap<_, _> = included
            .iter()
            .filter_map(|queued| queued.write.checkpoint.as_ref())
            .map(|(key, value)| (key.as_str(), value))
            .collect();
        let keys: Vec<_> = values.keys().copied().collect();
        Ok(self
            .0
            .read_many(&keys)
            .await
            .map_err(failure)?
            .into_iter()
            .zip(values.into_values())
            .map(|(previous, value)| Change {
                previous,
                value: value.clone(),
            })
            .collect())
    }

    pub async fn complete_reviews(
        &self,
        lens_id: &str,
        job: &Job,
        versions: &[ReviewVersion],
    ) -> Result<(), RepositoryError> {
        let expected: BTreeMap<_, _> = versions
            .iter()
            .map(|version| {
                Ok::<_, RepositoryError>((
                    review_key(lens_id, job, &version.execution_id)?,
                    version.content_version.clone(),
                ))
            })
            .collect::<Result<_, _>>()?;
        let keys: Vec<_> = expected.keys().map(String::as_str).collect();
        for attempt in 0..40 {
            let records = self.0.read_many(&keys).await.map_err(failure)?;
            let mut changes = Vec::new();
            for previous in records {
                if previous.value.is_null() {
                    continue;
                }
                let review: Review = decode(previous.value.clone())?;
                if Some(&review.content_version) == expected.get(&previous.head.key)
                    && !review.consolidated
                {
                    changes.push(Change {
                        previous,
                        value: document(Public(Review {
                            consolidated: true,
                            ..review
                        }))?,
                    });
                }
            }
            if changes.is_empty() {
                return Ok(());
            }
            match self.0.commit(changes).await {
                Ok(()) => return Ok(()),
                Err(Error::StateConflict) => (),
                Err(error) => return Err(failure(error)),
            }
            backoff(attempt).await;
        }
        Err(RepositoryError::Conflict)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::missing(None, false)]
    #[case::before_expiry(Some(1), true)]
    #[case::at_expiry(Some(0), false)]
    #[case::after_expiry(Some(-1), false)]
    fn checkpoint_requires_strictly_live_lease(
        #[case] remaining_microseconds: Option<i64>,
        #[case] expected: bool,
    ) {
        let now = "2026-03-01T12:00:00Z".parse::<DateTime<Utc>>().unwrap();
        let lease =
            remaining_microseconds.map(|remaining| now + chrono::Duration::microseconds(remaining));
        assert_eq!(live_lease(lease, now), expected);
    }
}
