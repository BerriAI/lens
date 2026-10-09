use crate::{
    Error, QueueOptions, criteria_key, queue_job, scheduled_window, validate_run, validate_settings,
};
use chrono::{DateTime, Utc};
use lens_contract::{
    investigations::{FindingUpdate, Lens, RunRequest, Scope},
    worker::{Finding, JobTrigger, LensSettings},
};

pub fn create_lens(
    settings: LensSettings,
    scope: Scope,
    now: DateTime<Utc>,
    lens_id: &str,
    job_id: &str,
) -> Result<Lens, Error> {
    validate_settings(&settings, now)?;
    let lens = Lens {
        id: lens_id.into(),
        scope,
        settings,
        revision: 1,
        version: 0,
        created_at: now,
        next_run_at: now,
        last_scan_at: None,
        jobs: Vec::new(),
        findings: Vec::new(),
        budget_month: now.format("%Y-%m").to_string(),
        spent: 0.0,
        reservations: Vec::new(),
        criteria_updated_at: None,
    };
    queue_job(&lens, now, job_id, QueueOptions::default())
}

pub fn update_settings(
    lens: &Lens,
    settings: LensSettings,
    now: DateTime<Utc>,
) -> Result<Lens, Error> {
    validate_settings(&settings, now)?;
    let changed = criteria_key(&lens.settings)? != criteria_key(&settings)?;
    Ok(Lens {
        settings,
        revision: lens.revision.checked_add(1).ok_or(Error::RevisionCount)?,
        criteria_updated_at: if changed {
            Some(now)
        } else {
            lens.criteria_updated_at
        },
        last_scan_at: if changed { None } else { lens.last_scan_at },
        ..lens.clone()
    })
}

pub fn manual_run(
    lens: &Lens,
    request: &RunRequest,
    now: DateTime<Utc>,
    job_id: &str,
) -> Result<Lens, Error> {
    validate_run(request, now)?;
    let settings = request.agent_name.as_ref().map_or_else(
        || request.settings.clone(),
        |agent| {
            Some(LensSettings {
                agent_name: agent.clone(),
                ..request.settings.as_ref().unwrap_or(&lens.settings).clone()
            })
        },
    );
    let window = match (request.start, request.end) {
        (Some(start), Some(end)) => Some((start, end)),
        _ if request.lookback_hours.is_none() && request.settings.is_none() => {
            Some(scheduled_window(lens, now)?)
        }
        _ => None,
    };
    queue_job(
        lens,
        now,
        job_id,
        QueueOptions {
            settings: settings.as_ref(),
            lookback_hours: request.lookback_hours,
            window,
            trigger: JobTrigger::Manual,
        },
    )
}

pub fn update_finding_status(lens: &Lens, finding_id: &str, update: &FindingUpdate) -> Lens {
    Lens {
        findings: lens
            .findings
            .iter()
            .map(|finding| {
                if finding.id == finding_id
                    || finding.merged_finding_ids.iter().any(|id| id == finding_id)
                {
                    Finding {
                        status: update.status,
                        reason: update.reason.clone(),
                        ..finding.clone()
                    }
                } else {
                    finding.clone()
                }
            })
            .collect(),
        ..lens.clone()
    }
}
