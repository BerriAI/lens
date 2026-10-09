use super::support::*;
use chrono::{DateTime, TimeDelta, Utc};
use lens_contract::{
    investigations::{Lens, Worker},
    worker::{Job, JobStatus, JobTrigger, LensSettings, Result as RunResult},
};
use lens_investigations::*;
use rstest::{fixture, rstest};
use serde_json::json;

#[rstest]
#[case::idle(true, None, None, Some(10))]
#[case::disabled(false, None, None, None)]
#[case::queued_disabled(false, Some(JobStatus::Queued), Some(5), Some(0))]
#[case::running(true, Some(JobStatus::Running), Some(5), Some(5))]
#[case::running_without_lease(true, Some(JobStatus::Running), None, Some(0))]
#[case::completed(true, Some(JobStatus::Completed), None, Some(10))]
#[case::failed_disabled(false, Some(JobStatus::Failed), None, None)]
#[case::cancelled(true, Some(JobStatus::Cancelled), None, Some(10))]
fn due_times(
    lens: Lens,
    job: Job,
    now: DateTime<Utc>,
    #[case] enabled: bool,
    #[case] status: Option<JobStatus>,
    #[case] lease: Option<i64>,
    #[case] due: Option<i64>,
) {
    let candidate = Lens {
        next_run_at: now + TimeDelta::minutes(10),
        settings: LensSettings {
            enabled,
            ..lens.settings.clone()
        },
        jobs: status
            .map(|status| Job {
                status,
                lease_until: lease.map(|value| now + TimeDelta::minutes(value)),
                ..job
            })
            .into_iter()
            .collect(),
        ..lens
    };
    assert_eq!(
        due_at(&candidate),
        due.map(|value| now + TimeDelta::minutes(value))
    );
}

#[rstest]
fn current_job_and_replacement_preserve_history_order(lens: Lens, job: Job) {
    let finished = Job {
        id: "old".into(),
        status: JobStatus::Completed,
        ..job.clone()
    };
    let next = Job {
        id: "next".into(),
        status: JobStatus::Running,
        ..job.clone()
    };
    let saved = Lens {
        jobs: vec![finished.clone(), job.clone(), next.clone()],
        ..lens
    };
    assert_eq!(current_job(&saved).unwrap().id, "job");
    let replacement = Job {
        stage: "Changed".into(),
        ..job.clone()
    };
    let updated = replace_job(&saved, replacement.clone());
    assert_eq!(json!(updated.jobs), json!([finished, replacement, next]));
    assert_eq!(
        json!(replace_job(
            &saved,
            Job {
                id: "absent".into(),
                ..job
            }
        )),
        json!(saved)
    );
}

#[rstest]
#[case::one_day(24)]
#[case::one_week(168)]
#[case::long_history(8760)]
fn first_scan_uses_full_configured_lookback(lens: Lens, now: DateTime<Utc>, #[case] hours: u64) {
    let configured = Lens {
        settings: LensSettings {
            lookback_hours: hours.try_into().unwrap(),
            ..lens.settings.clone()
        },
        ..lens
    };
    let queued = queue_job(&configured, now, "first", QueueOptions::default()).unwrap();
    assert_eq!(queued.jobs.len(), 1);
    let job = &queued.jobs[0];
    assert_eq!(
        (job.start, job.end),
        (
            now - TimeDelta::hours(hours as i64),
            now - TimeDelta::minutes(2)
        )
    );
    assert_eq!(job.trigger, JobTrigger::Schedule);
    assert_eq!(job.id, "first");
    assert_eq!(job.created_at, now);
    assert_eq!(job.revision, configured.revision);
    assert_eq!(job.status, JobStatus::Queued);
    assert_eq!(job.stage, "Queued");
}

#[rstest]
#[case::recent(-60,-60)]
#[case::old(-100_000,-1440)]
#[case::future(10,-2)]
#[case::settling(-1,-2)]
fn scheduled_scan_window_resumes_and_clamps(
    lens: Lens,
    now: DateTime<Utc>,
    #[case] previous: i64,
    #[case] start: i64,
) {
    let saved = Lens {
        last_scan_at: Some(now + TimeDelta::minutes(previous)),
        ..lens
    };
    assert_eq!(
        scheduled_window(&saved, now).unwrap(),
        (now + TimeDelta::minutes(start), now - TimeDelta::minutes(2))
    );
}

#[rstest]
#[case::queued(JobStatus::Queued)]
#[case::running(JobStatus::Running)]
fn active_queue_is_idempotent_with_frozen_settings(
    lens: Lens,
    job: Job,
    now: DateTime<Utc>,
    #[case] status: JobStatus,
) {
    let edited = Lens {
        jobs: vec![Job { status, ..job }],
        settings: LensSettings {
            model: "replacement".try_into().unwrap(),
            ..lens.settings.clone()
        },
        ..lens
    };
    let queued = queue_job(&edited, now, "duplicate", QueueOptions::default()).unwrap();
    assert_eq!(json!(queued), json!(edited));
    assert_eq!(queued.jobs[0].settings.model.as_str(), "analysis");
}

#[rstest]
#[case::settings_lookback(None, false, 72)]
#[case::explicit_lookback(Some(5), false, 5)]
#[case::window_precedence(Some(5), true, 3)]
fn one_off_settings_and_windows_leave_monitor_unchanged(
    lens: Lens,
    job: Job,
    now: DateTime<Utc>,
    #[case] hours: Option<u64>,
    #[case] exact_window: bool,
    #[case] expected_hours: i64,
) {
    let saved = Lens {
        jobs: vec![Job {
            status: JobStatus::Completed,
            ..job
        }],
        last_scan_at: Some(now - TimeDelta::minutes(60)),
        ..lens
    };
    let settings = LensSettings {
        lookback_hours: 72.try_into().unwrap(),
        concurrency: 3.try_into().unwrap(),
        sample_percent: 10.0,
        ..saved.settings.clone()
    };
    let queued = queue_job(
        &saved,
        now,
        "manual",
        QueueOptions {
            settings: Some(&settings),
            lookback_hours: hours.map(|value| value.try_into().unwrap()),
            window: exact_window.then_some((now - TimeDelta::hours(3), now - TimeDelta::hours(2))),
            trigger: JobTrigger::Manual,
        },
    )
    .unwrap();
    assert_eq!(json!(queued.settings), json!(saved.settings));
    assert_eq!(queued.jobs.len(), 1);
    assert_eq!(queued.jobs[0].start, now - TimeDelta::hours(expected_hours));
    assert_eq!(
        queued.jobs[0].end,
        if exact_window {
            now - TimeDelta::hours(2)
        } else {
            now - TimeDelta::minutes(2)
        }
    );
    assert_eq!(queued.jobs[0].trigger, JobTrigger::Manual);
    assert_eq!(json!(queued.jobs[0].settings), json!(settings));
}

#[rstest]
#[case::finding_with_error(true, false, "partial", TerminalStatus::Failed)]
#[case::assessment_with_error(false, true, "partial", TerminalStatus::Failed)]
#[case::total_failure(false, false, "failed", TerminalStatus::Failed)]
#[case::no_error(false, false, "", TerminalStatus::Completed)]
#[case::finding_without_error(true, false, "", TerminalStatus::Completed)]
#[case::assessment_without_error(false, true, "", TerminalStatus::Completed)]
fn results_with_errors_fail_even_after_partial_progress(
    #[case] finding: bool,
    #[case] assessable: bool,
    #[case] error: &str,
    #[case] expected: TerminalStatus,
) {
    let result: RunResult = decode(
        json!({"coverage":{},"error":error,"findings":if finding {vec![draft("run")]} else {vec![]},"assessments":[{"execution_id":"run","cannot_assess":!assessable}]}),
    );
    assert_eq!(result_status(&result), expected);
}

#[fixture]
fn incomplete_result() -> RunResult {
    decode(json!({
        "coverage": {"selected": 10, "screened": 7},
        "error": "Lens storage failed: investigation write conflict",
        "assessments": (0..7)
            .map(|index| json!({"execution_id": format!("run-{index}"), "cannot_assess": false}))
            .collect::<Vec<_>>()
    }))
}

#[rstest]
fn seven_of_ten_assessments_with_conflict_fail(incomplete_result: RunResult) {
    assert_eq!(result_status(&incomplete_result), TerminalStatus::Failed);
}

#[rstest]
#[case::scheduled((false,false,false),None,Some(-2))]
#[case::failed((true,false,false),Some(-180),Some(-180))]
#[case::manual((false,true,false),Some(-180),Some(-180))]
#[case::criteria_changed((false, false, true), None, None)]
#[case::never_backwards((false, false, false), Some(15), Some(15))]
fn only_successful_matching_schedule_advances_scan(
    lens: Lens,
    job: Job,
    now: DateTime<Utc>,
    #[case] mode: (bool, bool, bool),
    #[case] previous: Option<i64>,
    #[case] expected: Option<i64>,
) {
    let (failed, manual, changed) = mode;
    let saved = Lens {
        last_scan_at: previous.map(|value| now + TimeDelta::minutes(value)),
        settings: if changed {
            LensSettings {
                context: "New criteria".into(),
                ..lens.settings.clone()
            }
        } else {
            lens.settings.clone()
        },
        ..lens
    };
    let run = Job {
        trigger: if manual {
            JobTrigger::Manual
        } else {
            JobTrigger::Schedule
        },
        ..job
    };
    assert_eq!(
        next_scan_start(&saved, &run, failed).unwrap(),
        expected.map(|value| now + TimeDelta::minutes(value))
    );
}

#[rstest]
#[case::live_lease(0, "alpha", false)]
#[case::expired_lease(5, "alpha", true)]
#[case::late_worker(6, "alpha", true)]
#[case::foreign_team(6, "beta", false)]
fn lease_and_scope_control_reclaims(
    lens: Lens,
    worker: Worker,
    now: DateTime<Utc>,
    #[case] minutes: i64,
    #[case] team: &str,
    #[case] reclaimed: bool,
) {
    let queued = queue_job(&lens, now, "job", QueueOptions::default()).unwrap();
    let first = claim_job(&queued, &worker, now).unwrap();
    assert_eq!(first.jobs[0].attempts, 1);
    assert_eq!(first.jobs[0].stage, "Collecting executions");
    assert_eq!(first.jobs[0].lease_until, Some(now + TimeDelta::minutes(5)));
    let next_worker = Worker {
        id: "second".into(),
        scope: lens_contract::investigations::Scope {
            team_id: team.into(),
            ..Default::default()
        },
        ..worker
    };
    let next = claim_job(&first, &next_worker, now + TimeDelta::minutes(minutes)).unwrap();
    assert_eq!(
        next.jobs[0].worker_id.as_deref(),
        Some(if reclaimed { "second" } else { "worker" })
    );
    assert_eq!(next.jobs[0].attempts, if reclaimed { 2 } else { 1 });
}

#[rstest]
fn repeated_disconnects_fail_and_schedule_the_next_run(
    lens: Lens,
    worker: Worker,
    job: Job,
    now: DateTime<Utc>,
) {
    let saved = Lens {
        jobs: vec![Job {
            status: JobStatus::Running,
            attempts: 3,
            lease_until: Some(now),
            activities: vec![activity("one")],
            reading: decode(
                json!([{"execution_id":"one","trace_id":"t","agent":"a","started_at":now}]),
            ),
            ..job
        }],
        ..lens
    };
    let exhausted = claim_job(&saved, &worker, now).unwrap();
    let ended = &exhausted.jobs[0];
    assert_eq!(ended.status, JobStatus::Failed);
    assert_eq!(ended.stage, "Failed");
    assert_eq!(ended.error, "Worker disconnected repeatedly");
    assert_eq!(ended.finished_at, Some(now));
    assert!(ended.activities.is_empty() && ended.reading.is_empty());
    assert_eq!(exhausted.next_run_at, now + TimeDelta::minutes(15));
    assert!(current_job(&exhausted).is_none());
}

#[rstest]
fn reclaim_resets_live_reviews_but_keeps_result_state(
    lens: Lens,
    worker: Worker,
    job: Job,
    now: DateTime<Utc>,
) {
    let saved = Lens {
        jobs: vec![Job {
            status: JobStatus::Running,
            attempts: 1,
            lease_until: Some(now),
            reviews: vec![review(0)],
            reviewed: 9,
            activities: vec![activity("one")],
            reading: decode(
                json!([{"execution_id":"one","trace_id":"t","agent":"a","started_at":now}]),
            ),
            cost: 2.5,
            error: "previous".into(),
            ..job
        }],
        ..lens
    };
    let claimed = claim_job(&saved, &worker, now).unwrap();
    let run = &claimed.jobs[0];
    assert!(run.reviews.is_empty() && run.activities.is_empty() && run.reading.is_empty());
    assert_eq!((run.reviewed, run.attempts, run.cost), (0, 2, 2.5));
    assert_eq!(run.error, "previous");
}

#[rstest]
#[case::completed(TerminalStatus::Completed, JobStatus::Completed, "Complete")]
#[case::failed(TerminalStatus::Failed, JobStatus::Failed, "Failed")]
#[case::cancelled(TerminalStatus::Cancelled, JobStatus::Cancelled, "Cancelled")]
fn termination_clears_live_activity_only(
    job: Job,
    now: DateTime<Utc>,
    #[case] terminal: TerminalStatus,
    #[case] status: JobStatus,
    #[case] stage: &str,
) {
    let working = Job {
        activities: vec![activity("one")],
        reading: decode(
            json!([{"execution_id":"one","trace_id":"t","agent":"a","started_at":now}]),
        ),
        reviews: vec![review(0)],
        reviewed: 1,
        ..job
    };
    let ended = end_job(&working, terminal, now);
    assert_eq!(
        (ended.status, ended.stage.as_str(), ended.finished_at),
        (status, stage, Some(now))
    );
    assert!(ended.activities.is_empty() && ended.reading.is_empty());
    assert_eq!(json!(ended.reviews), json!(working.reviews));
    assert_eq!(ended.reviewed, 1);
}

#[rstest]
fn cancellation_and_idle_operations(lens: Lens, job: Job, worker: Worker, now: DateTime<Utc>) {
    assert_eq!(json!(cancel_job(&lens, now).unwrap()), json!(lens));
    assert_eq!(json!(claim_job(&lens, &worker, now).unwrap()), json!(lens));
    let saved = Lens {
        jobs: vec![job],
        ..lens
    };
    let cancelled = cancel_job(&saved, now).unwrap();
    assert_eq!(cancelled.jobs[0].status, JobStatus::Cancelled);
    assert_eq!(cancelled.next_run_at, now + TimeDelta::minutes(15));
}

#[rstest]
#[case::same_month("2026-01-31T23:59:59Z", 12.0, "2026-01")]
#[case::next_month("2026-02-01T00:00:00Z", 0.0, "2026-02")]
fn budget_month_renewal_preserves_job_costs_and_reservations(
    lens: Lens,
    job: Job,
    #[case] at: &str,
    #[case] spent: f64,
    #[case] month: &str,
) {
    let saved = Lens {
        spent: 12.0,
        jobs: vec![Job { cost: 2.0, ..job }],
        reservations: decode(json!([{"id":"r","job_id":"job","amount":1.0,"month":"2026-01"}])),
        ..lens
    };
    let updated = renew_budget(&saved, at.parse().unwrap());
    assert_eq!(
        (updated.spent, updated.budget_month.as_str()),
        (spent, month)
    );
    assert_eq!(json!(updated.jobs), json!(saved.jobs));
    assert_eq!(json!(updated.reservations), json!(saved.reservations));
}
