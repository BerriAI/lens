use super::support::*;
use chrono::{DateTime, TimeDelta, Utc};
use lens_contract::{
    investigations::{FindingUpdate, Lens, RunRequest, Scope},
    worker::{Finding, FindingStatus, Job, JobTrigger, LensSettings},
};
use lens_investigations::*;
use rstest::rstest;
use serde_json::json;

#[rstest]
fn creation_initializes_public_state_and_queues_first_scan(
    settings: LensSettings,
    now: DateTime<Utc>,
) {
    let scope = Scope {
        team_id: "team".into(),
        ..Default::default()
    };
    let created = create_lens(
        settings.clone(),
        scope.clone(),
        now,
        "created-lens",
        "first-job",
    )
    .unwrap();
    assert_eq!(created.id, "created-lens");
    assert_eq!(created.scope, scope);
    assert_eq!(json!(created.settings), json!(settings));
    assert_eq!(
        (created.revision, created.version, created.spent),
        (1, 0, 0.0)
    );
    assert_eq!((created.created_at, created.next_run_at), (now, now));
    assert!(created.last_scan_at.is_none() && created.criteria_updated_at.is_none());
    assert!(created.findings.is_empty() && created.reservations.is_empty());
    assert_eq!(created.budget_month, "2026-01");
    assert_eq!(created.jobs.len(), 1);
    assert_eq!(created.jobs[0].id, "first-job");
    assert_eq!(created.jobs[0].start, now - TimeDelta::hours(24));
    assert_eq!(created.jobs[0].trigger, JobTrigger::Schedule);
}

#[rstest]
#[case::operational(false)]
#[case::criteria(true)]
fn settings_updates_only_invalidate_matching_criteria(
    lens: Lens,
    job: Job,
    now: DateTime<Utc>,
    #[case] changed: bool,
) {
    let past = now - TimeDelta::days(1);
    let saved = Lens {
        last_scan_at: Some(past),
        criteria_updated_at: Some(past),
        jobs: vec![job],
        revision: 7,
        version: 12,
        ..lens
    };
    let settings = LensSettings {
        name: "Renamed".try_into().unwrap(),
        context: if changed {
            "New behavior".into()
        } else {
            saved.settings.context.clone()
        },
        ..saved.settings.clone()
    };
    let updated = update_settings(&saved, settings.clone(), now).unwrap();
    assert_eq!((updated.revision, updated.version), (8, 12));
    assert_eq!(
        updated.criteria_updated_at,
        Some(if changed { now } else { past })
    );
    assert_eq!(
        updated.last_scan_at,
        if changed { None } else { Some(past) }
    );
    assert_eq!(json!(updated.settings), json!(settings));
    assert_eq!(json!(updated.jobs), json!(saved.jobs));
}

#[rstest]
fn identical_settings_still_advance_revision(lens: Lens, now: DateTime<Utc>) {
    let updated = update_settings(&lens, lens.settings.clone(), now).unwrap();
    assert_eq!(updated.revision, 2);
    assert!(updated.criteria_updated_at.is_none() && updated.last_scan_at.is_none());
    assert_eq!(
        json!(Lens {
            revision: lens.revision,
            ..updated
        }),
        json!(lens)
    );
}

#[rstest]
fn settings_validation_and_revision_overflow_fail_without_changes(lens: Lens, now: DateTime<Utc>) {
    let invalid = LensSettings {
        checks: vec![],
        ..lens.settings.clone()
    };
    assert!(matches!(
        update_settings(&lens, invalid.clone(), now),
        Err(Error::MissingChecks)
    ));
    assert!(matches!(
        create_lens(invalid, lens.scope.clone(), now, "id", "job"),
        Err(Error::MissingChecks)
    ));
    let overflowing = Lens {
        revision: i64::MAX,
        ..lens
    };
    assert!(matches!(
        update_settings(&overflowing, overflowing.settings.clone(), now),
        Err(Error::RevisionCount)
    ));
}

#[rstest]
#[case::default(false,None,None,false,-1)]
#[case::agent_only(false,None,Some("other"),false,-1)]
#[case::settings(true,None,None,false,-72)]
#[case::settings_with_agent(true,None,Some("other"),false,-72)]
#[case::lookback(false,Some(5),None,false,-5)]
#[case::lookback_over_settings(true,Some(5),Some("other"),false,-5)]
#[case::explicit_window(true,Some(5),Some("other"),true,-3)]
fn manual_runs_apply_only_requested_overrides(
    lens: Lens,
    now: DateTime<Utc>,
    #[case] settings: bool,
    #[case] lookback: Option<u64>,
    #[case] agent: Option<&str>,
    #[case] explicit: bool,
    #[case] start_hours: i64,
) {
    let saved = Lens {
        last_scan_at: Some(now - TimeDelta::hours(1)),
        ..lens
    };
    let request = RunRequest {
        settings: settings.then(|| LensSettings {
            lookback_hours: 72.try_into().unwrap(),
            agent_name: "configured".into(),
            ..saved.settings.clone()
        }),
        lookback_hours: lookback.map(|value| value.try_into().unwrap()),
        agent_name: agent.map(str::to_owned),
        start: explicit.then_some(now - TimeDelta::hours(3)),
        end: explicit.then_some(now - TimeDelta::hours(2)),
    };
    let updated = manual_run(&saved, &request, now, "manual").unwrap();
    let job = &updated.jobs[0];
    assert_eq!(job.trigger, JobTrigger::Manual);
    assert_eq!(job.id, "manual");
    assert_eq!(job.start, now + TimeDelta::hours(start_hours));
    assert_eq!(
        job.end,
        if explicit {
            now - TimeDelta::hours(2)
        } else {
            now - TimeDelta::minutes(2)
        }
    );
    assert_eq!(
        job.settings.agent_name,
        agent.unwrap_or(if settings { "configured" } else { "" })
    );
    assert_eq!(
        job.settings.lookback_hours.get(),
        if settings { 72 } else { 24 }
    );
    assert_eq!(json!(updated.settings), json!(saved.settings));
}

#[rstest]
fn manual_run_keeps_active_job_and_validates_windows(lens: Lens, job: Job, now: DateTime<Utc>) {
    let saved = Lens {
        jobs: vec![job],
        ..lens
    };
    assert_eq!(
        json!(manual_run(&saved, &RunRequest::default(), now, "duplicate").unwrap()),
        json!(saved)
    );
    assert!(matches!(
        manual_run(
            &saved,
            &RunRequest {
                start: Some(now),
                ..Default::default()
            },
            now,
            "invalid"
        ),
        Err(Error::IncompleteWindow)
    ));
}

#[rstest]
#[case::direct("first", true)]
#[case::alias("alias", true)]
#[case::missing("missing", false)]
fn feedback_updates_direct_and_merged_identities_only(
    lens: Lens,
    now: DateTime<Utc>,
    #[case] id: &str,
    #[case] changed: bool,
) {
    let first = Finding {
        merged_finding_ids: vec!["alias".into()],
        ..merge_finding(&lens, &draft("first"), 1, now, "first", None, true).unwrap()
    };
    let second = merge_finding(&lens, &draft("second"), 1, now, "second", None, true).unwrap();
    let saved = Lens {
        findings: vec![first, second.clone()],
        ..lens
    };
    let update = FindingUpdate {
        status: FindingStatus::Resolved,
        reason: "Fixed".into(),
    };
    let updated = update_finding_status(&saved, id, &update);
    assert_eq!(
        updated.findings[0].status,
        if changed {
            FindingStatus::Resolved
        } else {
            FindingStatus::Open
        }
    );
    assert_eq!(
        updated.findings[0].reason,
        if changed { "Fixed" } else { "" }
    );
    assert_eq!(json!(updated.findings[1]), json!(second));
    assert_eq!(updated.version, saved.version);
    assert_eq!(updated.revision, saved.revision);
}
