use chrono::Utc;
use lens_contract::worker::{Job, JobStatus};
use lens_investigations::{AnalysisCheckpoint, CheckpointError, RepositoryError, current_job};
use tokio::time::{Instant, timeout_at};

use super::{
    Investigations, StoredLens, decode, document, failure, key, workers::backoff,
    writes::WRITE_RETRY_WINDOW,
};
use crate::state::Change;

impl Investigations {
    pub async fn analysis_checkpoint(
        &self,
        lens_id: &str,
        fingerprint: &str,
    ) -> Result<Option<AnalysisCheckpoint>, RepositoryError> {
        let snapshot = self
            .0
            .read(&key("analysis-checkpoint", (lens_id, fingerprint))?)
            .await
            .map_err(failure)?;
        if snapshot.value.is_null() {
            return Ok(None);
        }
        decode(snapshot.value).map(Some)
    }

    pub async fn save_analysis_checkpoint(
        &self,
        lens_id: &str,
        assigned: &Job,
        fingerprint: &str,
        checkpoint: &AnalysisCheckpoint,
    ) -> Result<AnalysisCheckpoint, CheckpointError> {
        let key = key("analysis-checkpoint", (lens_id, fingerprint))?;
        let value = document(checkpoint)?;
        let deadline = Instant::now() + WRITE_RETRY_WINDOW;
        let mut unconfirmed = false;
        for attempt in 0..40 {
            if Instant::now() >= deadline {
                break;
            }
            if attempt > 0 {
                backoff(attempt - 1).await;
            }
            let writer = match timeout_at(deadline, self.writer(lens_id)).await {
                Ok(Ok(writer)) => writer,
                Ok(Err(RepositoryError::Conflict)) => continue,
                Ok(Err(error)) => return Err(error.into()),
                Err(_) => break,
            };
            let snapshot = self.snapshot(lens_id).await?;
            if snapshot.value.is_null() {
                return Err(CheckpointError::Ownership);
            }
            let current: StoredLens = decode(snapshot.value)?;
            let job = current_job(&current.lens).ok_or(CheckpointError::Ownership)?;
            if job.id != assigned.id
                || job.worker_id != assigned.worker_id
                || job.attempts != assigned.attempts
                || job.status != JobStatus::Running
                || job.lease_until.is_none_or(|lease| lease <= Utc::now())
            {
                return Err(CheckpointError::Ownership);
            }
            let previous = self.0.read(&key).await.map_err(failure)?;
            if !previous.value.is_null() {
                return Ok(decode(previous.value)?);
            }
            match writer
                .commit(vec![Change {
                    previous,
                    value: value.clone(),
                }])
                .await
            {
                Ok(()) => return Ok(checkpoint.clone()),
                Err(RepositoryError::Conflict) => unconfirmed = false,
                Err(RepositoryError::WriteUnconfirmed) => unconfirmed = true,
                Err(error) => return Err(error.into()),
            }
        }
        Err(if unconfirmed {
            RepositoryError::WriteUnconfirmed
        } else {
            RepositoryError::Conflict
        }
        .into())
    }
}
