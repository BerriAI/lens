use crate::{
    Error, can_access, criteria_key,
    settings::{calendar, interval_end, lookback_start},
};
use chrono::{DateTime, TimeDelta, Utc};
use lens_contract::{
    investigations::{Lens, Worker},
    worker::{Coverage, Job, JobStatus, JobTrigger, LensSettings, Result as InvestigationResult},
};
use std::num::NonZeroU64;

#[derive(Clone, Copy, Debug, Default)]
pub struct QueueOptions<'a> {
    pub lookback_hours: Option<NonZeroU64>,
    pub settings: Option<&'a LensSettings>,
    pub window: Option<(DateTime<Utc>, DateTime<Utc>)>,
    pub trigger: JobTrigger,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalStatus {
    Completed,
    Failed,
    Cancelled,
}

pub fn current_job(lens: &Lens) -> Option<&Job> {
    lens.jobs
        .iter()
        .find(|job| matches!(job.status, JobStatus::Queued | JobStatus::Running))
}

pub fn due_at(lens: &Lens) -> Option<DateTime<Utc>> {
    match current_job(lens) {
        Some(job) if job.status == JobStatus::Queued => Some(job.created_at),
        Some(job) => Some(job.lease_until.unwrap_or(job.created_at)),
        None => lens.settings.enabled.then_some(lens.next_run_at),
    }
}

pub fn replace_job(lens: &Lens, job: Job) -> Lens {
    Lens {
        jobs: lens
            .jobs
            .iter()
            .map(|old| {
                if old.id == job.id {
                    job.clone()
                } else {
                    old.clone()
                }
            })
            .collect(),
        ..lens.clone()
    }
}

pub fn scheduled_window(
    lens: &Lens,
    now: DateTime<Utc>,
) -> Result<(DateTime<Utc>, DateTime<Utc>), Error> {
    let end = calendar(now.checked_sub_signed(TimeDelta::minutes(2)), "Lookback")?;
    let floor = lookback_start(now, lens.settings.lookback_hours.get())?;
    let start = lens
        .last_scan_at
        .map_or(floor, |previous| previous.max(floor));
    Ok((start.min(end), end))
}

pub fn next_scan_start(
    lens: &Lens,
    job: &Job,
    failed: bool,
) -> Result<Option<DateTime<Utc>>, Error> {
    if failed
        || job.trigger == JobTrigger::Manual
        || criteria_key(&lens.settings)? != criteria_key(&job.settings)?
    {
        return Ok(lens.last_scan_at);
    }
    Ok(Some(lens.last_scan_at.unwrap_or(job.end).max(job.end)))
}

pub fn queue_job(
    lens: &Lens,
    now: DateTime<Utc>,
    job_id: &str,
    options: QueueOptions<'_>,
) -> Result<Lens, Error> {
    if current_job(lens).is_some() {
        return Ok(lens.clone());
    }
    let settings = options.settings.unwrap_or(&lens.settings);
    let hours = options
        .lookback_hours
        .or(options.settings.map(|value| value.lookback_hours));
    let (start, end) = match (options.window, hours) {
        (Some(window), _) => window,
        (None, Some(hours)) => (
            lookback_start(now, hours.get())?,
            calendar(now.checked_sub_signed(TimeDelta::minutes(2)), "Lookback")?,
        ),
        (None, None) => scheduled_window(lens, now)?,
    };
    let job = Job {
        id: job_id.into(),
        status: JobStatus::Queued,
        stage: "Queued".into(),
        created_at: now,
        start,
        end,
        settings: settings.clone(),
        revision: lens.revision,
        worker_id: None,
        lease_until: None,
        attempts: 0,
        finished_at: None,
        coverage: Coverage::default(),
        error: String::new(),
        sample: None,
        cost: 0.0,
        findings: None,
        assessments: Vec::new(),
        steps: Vec::new(),
        reviews: Vec::new(),
        reviewed: 0,
        reading: Vec::new(),
        activities: Vec::new(),
        trigger: options.trigger,
        review_versions: Vec::new(),
    };
    Ok(Lens {
        jobs: vec![job],
        ..lens.clone()
    })
}

pub fn result_status(result: &InvestigationResult) -> TerminalStatus {
    if !result.error.is_empty() {
        TerminalStatus::Failed
    } else {
        TerminalStatus::Completed
    }
}

pub fn end_job(job: &Job, status: TerminalStatus, now: DateTime<Utc>) -> Job {
    let (status, stage) = match status {
        TerminalStatus::Completed => (JobStatus::Completed, "Complete"),
        TerminalStatus::Failed => (JobStatus::Failed, "Failed"),
        TerminalStatus::Cancelled => (JobStatus::Cancelled, "Cancelled"),
    };
    Job {
        status,
        stage: stage.into(),
        finished_at: Some(now),
        reading: Vec::new(),
        activities: Vec::new(),
        ..job.clone()
    }
}

pub fn cancel_job(lens: &Lens, now: DateTime<Utc>) -> Result<Lens, Error> {
    let Some(job) = current_job(lens) else {
        return Ok(lens.clone());
    };
    Ok(Lens {
        next_run_at: interval_end(now, lens.settings.interval_minutes.get())?,
        ..replace_job(lens, end_job(job, TerminalStatus::Cancelled, now))
    })
}

pub fn claim_job(lens: &Lens, worker: &Worker, now: DateTime<Utc>) -> Result<Lens, Error> {
    let Some(job) = current_job(lens) else {
        return Ok(lens.clone());
    };
    if !can_access(&worker.scope, &lens.scope)
        || (job.status == JobStatus::Running && job.lease_until.is_some_and(|lease| lease > now))
    {
        return Ok(lens.clone());
    }
    if job.attempts >= 3 {
        return Ok(Lens {
            next_run_at: interval_end(now, lens.settings.interval_minutes.get())?,
            ..replace_job(
                lens,
                Job {
                    error: "Worker disconnected repeatedly".into(),
                    ..end_job(job, TerminalStatus::Failed, now)
                },
            )
        });
    }
    Ok(replace_job(
        lens,
        Job {
            status: JobStatus::Running,
            stage: "Collecting executions".into(),
            worker_id: Some(worker.id.clone()),
            lease_until: Some(interval_end(now, 5)?),
            attempts: job.attempts + 1,
            reviews: Vec::new(),
            reviewed: 0,
            reading: Vec::new(),
            activities: Vec::new(),
            ..job.clone()
        },
    ))
}

pub fn renew_budget(lens: &Lens, now: DateTime<Utc>) -> Lens {
    let month = now.format("%Y-%m").to_string();
    if lens.budget_month == month {
        return lens.clone();
    }
    Lens {
        budget_month: month,
        spent: 0.0,
        ..lens.clone()
    }
}
