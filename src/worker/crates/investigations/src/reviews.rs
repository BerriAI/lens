use crate::{
    Error, analysis_checks,
    settings::{interval_end, trimmed},
};
use chrono::{DateTime, Utc};
use lens_contract::{
    investigations::{Lens, ReviewPage},
    worker::{Activity, Extraction, Job, LensSettings, Progress, Review, Sample, Step, StepKind},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{self, Write};

struct PythonArrays;

impl serde_json::ser::Formatter for PythonArrays {
    fn begin_array_value<W: Write + ?Sized>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> io::Result<()> {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }
}

pub fn criteria_key(settings: &LensSettings) -> Result<String, Error> {
    let checks = analysis_checks(settings)?;
    let mut criteria: Vec<_> = checks
        .iter()
        .map(|check| (check.id.as_str(), trimmed(&check.instruction)))
        .collect();
    criteria.sort_unstable();
    let payload = (
        trimmed(&settings.context),
        criteria,
        settings.model.as_str(),
    );
    let mut serializer = serde_json::Serializer::with_formatter(Vec::new(), PythonArrays);
    payload.serialize(&mut serializer)?;
    Ok(format!("{:x}", Sha256::digest(serializer.into_inner())))
}

pub fn map_extraction(extraction: &Extraction, identity: impl Fn(&str) -> String) -> Extraction {
    Extraction {
        observations: extraction
            .observations
            .iter()
            .map(|observation| lens_contract::worker::Observation {
                evidence: observation
                    .evidence
                    .iter()
                    .map(|quote| lens_contract::worker::Evidence {
                        execution_id: identity(&quote.execution_id),
                        ..quote.clone()
                    })
                    .collect(),
                ..observation.clone()
            })
            .collect(),
        ..extraction.clone()
    }
}

pub fn map_review(review: &Review, identity: impl Fn(&str) -> String) -> Review {
    Review {
        execution_id: identity(&review.execution_id),
        extraction: review
            .extraction
            .as_ref()
            .map(|value| map_extraction(value, &identity)),
        ..review.clone()
    }
}

pub fn add_step(job: &Job, step: Step) -> Job {
    Job {
        steps: job.steps.iter().cloned().chain([step]).collect(),
        ..job.clone()
    }
}

pub fn add_review(job: &Job, review: Option<&Review>) -> Result<Job, Error> {
    let Some(review) = review else {
        return Ok(job.clone());
    };
    let summary = Review {
        extraction: None,
        content_version: String::new(),
        ..review.clone()
    };
    Ok(Job {
        reviews: job.reviews.iter().cloned().chain([summary]).collect(),
        reviewed: job.reviewed.checked_add(1).ok_or(Error::ReviewCount)?,
        ..job.clone()
    })
}

pub fn update_activity(activities: &[Activity], activity: Option<&Activity>) -> Vec<Activity> {
    let Some(activity) = activity else {
        return activities.to_vec();
    };
    if activity.finished {
        return activities
            .iter()
            .filter(|item| item.id != activity.id)
            .cloned()
            .collect();
    }
    if activities.iter().any(|item| item.id == activity.id) {
        return activities
            .iter()
            .map(|item| {
                if item.id == activity.id {
                    activity.clone()
                } else {
                    item.clone()
                }
            })
            .collect();
    }
    activities
        .iter()
        .cloned()
        .chain([activity.clone()])
        .collect()
}

pub fn apply_progress(job: &Job, progress: &Progress, now: DateTime<Utc>) -> Result<Job, Error> {
    let renewed = add_review(
        &Job {
            stage: progress.stage.clone().unwrap_or_else(|| job.stage.clone()),
            coverage: progress
                .coverage
                .clone()
                .unwrap_or_else(|| job.coverage.clone()),
            lease_until: Some(interval_end(now, 5)?),
            reading: progress
                .reading
                .clone()
                .unwrap_or_else(|| job.reading.clone()),
            activities: update_activity(&job.activities, progress.activity.as_ref()),
            ..job.clone()
        },
        progress.review.as_ref(),
    )?;
    if renewed.stage == job.stage {
        return Ok(renewed);
    }
    let step = Step {
        at: now,
        kind: StepKind::Stage,
        label: renewed.stage.clone().try_into()?,
        model: Default::default(),
        purpose: Default::default(),
        prompt_tokens: 0,
        completion_tokens: 0,
        cost: 0.0,
    };
    Ok(add_step(&renewed, step))
}

pub fn without_attributes(sample: &Sample) -> Sample {
    Sample {
        executions: sample
            .executions
            .iter()
            .map(|execution| lens_contract::worker::Execution {
                metadata: Vec::new(),
                ..execution.clone()
            })
            .collect(),
        ..sample.clone()
    }
}

pub fn summarized_job(job: &Job) -> Job {
    Job {
        reviews: Vec::new(),
        sample: job.sample.as_ref().map(without_attributes),
        ..job.clone()
    }
}

pub fn summarized(lens: &Lens) -> Lens {
    Lens {
        jobs: lens.jobs.iter().map(summarized_job).collect(),
        ..lens.clone()
    }
}

pub fn reviews_after(job: &Job, after: i64) -> ReviewPage {
    let first_kept = i128::from(job.reviewed) - job.reviews.len() as i128;
    let offset = (i128::from(after) - first_kept)
        .max(0)
        .min(job.reviews.len() as i128) as usize;
    ReviewPage {
        reviews: job.reviews[offset..].to_vec(),
        reviewed: job.reviewed,
    }
}
