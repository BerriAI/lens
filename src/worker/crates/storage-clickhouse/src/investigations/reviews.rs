use std::{collections::BTreeMap, time::Instant};

use chrono::{DateTime, Utc};
use lens_contract::{
    investigations::{Lens, Public},
    worker::{Job, JobStatus, Progress, Review, ReviewVersion},
};
use lens_investigations::{
    CheckpointError, RepositoryError, apply_progress, criteria_key, current_job, replace_job,
};

use super::{Investigations, StoredLens, decode, document, failure, key, workers::backoff};
use crate::{Error, state::Change};

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
        for attempt in 0..40 {
            let previous = self.snapshot(lens_id).await?;
            if previous.value.is_null() {
                return Ok(None);
            }
            let current: StoredLens = decode(previous.value.clone())?;
            let now = progress_time(now, started)?;
            let job = current_job(&current.lens).ok_or(CheckpointError::Ownership)?;
            if job.id != assigned.id
                || job.worker_id != assigned.worker_id
                || job.attempts != assigned.attempts
                || job.status != JobStatus::Running
                || !live_lease(job.lease_until, now)
            {
                return Err(CheckpointError::Ownership);
            }
            let candidate = replace_job(&current.lens, apply_progress(job, body, now)?);
            let checkpoint = match &cache {
                Some((key, value)) => Some(Change {
                    previous: self.0.read(key).await.map_err(failure)?,
                    value: value.clone(),
                }),
                None => None,
            };
            match self
                .publish_lens(previous, &current.lens, &candidate, checkpoint)
                .await
            {
                Ok(lens) => return Ok(Some(lens)),
                Err(RepositoryError::Conflict) => (),
                Err(error) => return Err(error.into()),
            }
            backoff(attempt).await;
        }
        Ok(None)
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
