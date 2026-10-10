use chrono::{DateTime, Utc};
use futures_util::future::BoxFuture;
use lens_contract::{
    investigations::Lens,
    worker::{Job, JobStatus, Progress, Sample},
};
use lens_investigations::{
    LensRepository, WorkerRepository, analysis_checks, can_access, current_job, replace_job,
};

use super::{LocalControl, rejected};
use crate::{Error, SampleRequest, control::JobBackend, wire};

#[derive(Clone)]
pub(super) struct LocalJob {
    pub control: LocalControl,
    pub lens_id: String,
    pub job_id: String,
    pub attempt: i64,
    pub worker_id: String,
}

impl LocalJob {
    pub(super) fn active<'a>(&self, lens: &'a Lens, now: DateTime<Utc>) -> Result<&'a Job, Error> {
        current_job(lens)
            .filter(|job| {
                job.id == self.job_id
                    && job.worker_id.as_deref() == Some(&self.worker_id)
                    && job.attempts == self.attempt
                    && job.status == JobStatus::Running
                    && job.lease_until.is_some_and(|lease| lease > now)
            })
            .ok_or_else(|| rejected(409, "This worker no longer owns the job"))
    }

    pub(super) async fn lens(&self) -> Result<Lens, Error> {
        let worker = self.control.worker().await?;
        if worker.id != self.worker_id {
            return Err(rejected(409, "This worker no longer owns the job"));
        }
        self.control
            .repository
            .get(&self.lens_id)
            .await?
            .filter(|lens| can_access(&worker.scope, &lens.scope))
            .ok_or_else(|| rejected(404, "Lens not found"))
    }

    pub(super) async fn assigned(&self) -> Result<(Lens, Job), Error> {
        let lens = self.lens().await?;
        let job = self.active(&lens, Utc::now())?.clone();
        Ok((lens, job))
    }

    async fn read_sample(&self) -> Result<Sample, Error> {
        let (lens, job) = self.assigned().await?;
        if let Some(sample) = job.sample {
            return Ok(sample);
        }
        let selection = (&job.settings).into();
        let sizes = [10_000, 5_000, 2_500, 1_250, 625, 312, 156, 100];
        let mut size = 0;
        let mut cursor = String::new();
        let mut executions = Vec::new();
        let mut totals = None;
        loop {
            let page = self
                .control
                .sources
                .sample(
                    &lens.scope,
                    SampleRequest {
                        selection: &selection,
                        start: job
                            .start
                            .timestamp_millis()
                            .try_into()
                            .map_err(|_| Error::InvalidRequest)?,
                        end: job
                            .end
                            .timestamp_millis()
                            .try_into()
                            .map_err(|_| Error::InvalidRequest)?,
                        offset: 0,
                        page_size: sizes[size],
                        preview: false,
                        cursor: &cursor,
                    },
                )
                .await;
            let page = match page {
                Err(Error::StateStorage(litellm_storage_clickhouse::Error::ResponseTooLarge))
                    if size + 1 < sizes.len() =>
                {
                    size += 1;
                    continue;
                }
                result => result?,
            };
            let (_, selected) = *totals.get_or_insert((page.eligible, page.selected));
            executions.extend(page.executions);
            if page.next_cursor.is_none() || executions.len() as i64 >= selected {
                break;
            }
            let next = page.next_cursor.ok_or(Error::InvalidRequest)?;
            if next == cursor {
                return Err(Error::EvidenceCursorRepeated);
            }
            cursor = next;
        }
        let selected = Sample {
            eligible: totals.map_or(0, |value| value.0),
            selected: executions.len().try_into().map_err(|_| Error::TooLarge)?,
            executions,
            next_cursor: None,
            next_offset: None,
        };
        let updated = self
            .control
            .update(&self.lens_id, |lens| {
                let active = self.active(lens, Utc::now())?;
                if active.sample.is_some() {
                    return Ok(lens.clone());
                }
                Ok(replace_job(
                    lens,
                    Job {
                        sample: Some(selected.clone()),
                        ..active.clone()
                    },
                ))
            })
            .await?;
        self.active(&updated, Utc::now())?
            .sample
            .clone()
            .ok_or_else(|| rejected(409, "Could not freeze the sample"))
    }

    async fn save_progress(&self, body: &Progress) -> Result<(), Error> {
        let (_, job) = self.assigned().await?;
        if let Some(review) = &body.review {
            if job.sample.as_ref().is_none_or(|sample| {
                !sample
                    .executions
                    .iter()
                    .any(|execution| execution.id == review.execution_id)
            }) {
                return Err(rejected(422, "Review references a trace outside this job"));
            }
            let checks = analysis_checks(&job.settings)?;
            if review.extraction.as_ref().is_some_and(|extraction| {
                extraction.observations.iter().any(|observation| {
                    !checks
                        .iter()
                        .any(|check| check.id.as_str() == observation.check_id)
                        || observation
                            .evidence
                            .iter()
                            .any(|quote| quote.execution_id != review.execution_id)
                })
            }) {
                return Err(rejected(
                    422,
                    "Cached review must use enabled checks and only its assigned trace",
                ));
            }
        }
        self.control
            .repository
            .progress(&self.lens_id, &job, body, Utc::now())
            .await
            .map_err(|error| match error {
                lens_investigations::CheckpointError::Ownership => {
                    rejected(409, "This worker no longer owns the job")
                }
                error => error.into(),
            })?
            .ok_or_else(|| rejected(409, "Lens changed concurrently; retry the operation"))?;
        self.control
            .repository
            .heartbeat(&self.worker_id, Utc::now())
            .await?;
        Ok(())
    }
}

impl JobBackend for LocalJob {
    fn sample(&self) -> BoxFuture<'_, Result<Sample, Error>> {
        response(self.read_sample())
    }
    fn reviews(&self) -> BoxFuture<'_, Result<Vec<wire::Review>, Error>> {
        response(async move {
            let (_, job) = self.assigned().await?;
            Ok(self.control.repository.reviews(&self.lens_id, &job).await?)
        })
    }
    fn content<'a>(
        &'a self,
        execution_id: &'a str,
        cursor: &'a str,
        offset: usize,
    ) -> BoxFuture<'a, Result<wire::ExecutionContent, Error>> {
        response(async move {
            let (lens, job) = self.assigned().await?;
            let execution = job
                .sample
                .as_ref()
                .and_then(|sample| {
                    sample
                        .executions
                        .iter()
                        .find(|execution| execution.id == execution_id)
                })
                .ok_or_else(|| rejected(404, "Execution is outside this job's sample"))?;
            self.control
                .sources
                .content(
                    &lens.scope,
                    execution,
                    cursor,
                    Some(offset.try_into().map_err(|_| Error::InvalidRequest)?),
                )
                .await
        })
    }
    fn model<'a>(
        &'a self,
        body: &'a wire::ModelRequest,
    ) -> BoxFuture<'a, Result<wire::ModelResult, Error>> {
        response(self.analyze(body))
    }
    fn progress<'a>(&'a self, body: &'a Progress) -> BoxFuture<'a, Result<(), Error>> {
        response(self.save_progress(body))
    }
    fn finish<'a>(&'a self, result: &'a wire::Result) -> BoxFuture<'a, Result<(), Error>> {
        response(self.save_result(result))
    }
}

fn response<'a, T: Send + 'a>(
    future: impl std::future::Future<Output = Result<T, Error>> + Send + 'a,
) -> BoxFuture<'a, Result<T, Error>> {
    Box::pin(async move { future.await.map_err(super::backend_error) })
}
