use chrono::Utc;
use lens_contract::{
    investigations::Lens,
    worker::{Job, JobStatus},
};
use lens_investigations::{AnalysisCheckpoint, CheckpointError, RepositoryError, current_job};
use serde_json::Value;
use tokio::sync::oneshot;

use super::{
    Investigations, decode, document, failure, key,
    pending::{Edit, Outcome, Record},
};

struct CheckpointEdit {
    assigned: Job,
    checkpoint: AnalysisCheckpoint,
    record: Record,
    reply: oneshot::Sender<Result<AnalysisCheckpoint, CheckpointError>>,
}

impl CheckpointEdit {
    fn owns(&self, lens: &Lens) -> bool {
        current_job(lens).is_some_and(|job| {
            job.id == self.assigned.id
                && job.worker_id == self.assigned.worker_id
                && job.attempts == self.assigned.attempts
                && job.status == JobStatus::Running
                && job.lease_until.is_some_and(|lease| lease > Utc::now())
        })
    }
}

impl Edit for CheckpointEdit {
    fn abandoned(&self) -> bool {
        self.reply.is_closed()
    }

    fn record(&self) -> Option<&Record> {
        Some(&self.record)
    }

    fn replays_unconfirmed(&self) -> bool {
        true
    }

    fn apply(self: Box<Self>, candidate: Lens) -> (Lens, Option<Box<dyn Edit>>) {
        if self.owns(&candidate) {
            return (candidate, Some(self));
        }
        let _ = self.reply.send(Err(CheckpointError::Ownership));
        (candidate, None)
    }

    fn replacing(self: Box<Self>, stored: Value) -> Option<Box<dyn Edit>> {
        let _ = self
            .reply
            .send(decode(stored).map_err(CheckpointError::from));
        None
    }

    fn answer(self: Box<Self>, outcome: Outcome<'_>) {
        let _ = self.reply.send(match outcome {
            Outcome::Saved(_) => Ok(self.checkpoint),
            Outcome::Missing => Err(CheckpointError::Ownership),
            Outcome::Failed(error) => Err(error.into()),
        });
    }
}

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
        let record = Record {
            key: key("analysis-checkpoint", (lens_id, fingerprint))?,
            value: document(checkpoint)?,
        };
        self.publish_edit(lens_id, |reply| {
            Box::new(CheckpointEdit {
                assigned: assigned.clone(),
                checkpoint: checkpoint.clone(),
                record,
                reply,
            })
        })
        .await
    }
}
