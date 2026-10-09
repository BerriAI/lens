#[path = "evals/support.rs"]
mod support;

use chrono::{DateTime, Duration, Utc};
use lens_contract::eval::{CaseResult, EvalRun, RunStatus};
use lens_evals::{Evaluation, RunError, RunRepository, StoredRun};
use litellm_storage_clickhouse::state::Change;
use rstest::rstest;
use serde_json::json;
use support::{
    Database, claim_concurrently, create_concurrently, database, evaluation, isolated_database,
    now, record_key, result, run, scoring, seed_runs, submit_concurrently,
};

#[rstest]
#[tokio::test]
async fn creation_roundtrips_frozen_cases_and_idempotent_retry_returns_latest_run(
    #[future(awt)] database: Database,
    run: StoredRun,
    result: CaseResult,
    now: DateTime<Utc>,
) {
    let repository = database.repository();
    assert!(repository.get(&run.run.id).await.unwrap().is_none());
    assert_eq!(repository.create("key", &run).await.unwrap(), run);
    repository
        .submit(&run.run.id, "case-a", 0, &result, now)
        .await
        .unwrap();
    let current = repository.get(&run.run.id).await.unwrap().unwrap();
    let retry = StoredRun {
        run: EvalRun {
            id: "another-id".into(),
            ..run.run.clone()
        },
        ..run.clone()
    };
    assert_eq!(repository.create("key", &retry).await.unwrap(), current);
    assert_eq!(repository.list().await.unwrap(), vec![current.clone()]);
    assert_eq!(current.cases, run.cases);
    assert_eq!(
        database
            .store
            .read(&record_key("eval-run", &[&run.run.id]))
            .await
            .unwrap()
            .value,
        serde_json::to_value(current).unwrap()
    );
}

#[rstest]
#[tokio::test]
async fn idempotency_keys_are_scoped_to_team_and_reject_changed_specs(
    #[future(awt)] database: Database,
    run: StoredRun,
) {
    let repository = database.repository();
    repository.create("key", &run).await.unwrap();
    let changed = StoredRun {
        spec: lens_contract::eval::CreateEvalRun {
            version: "new-build".into(),
            ..run.spec.clone()
        },
        ..run.clone()
    };
    assert!(matches!(
        repository.create("key", &changed).await,
        Err(RunError::IdempotencyConflict)
    ));
    let other = StoredRun {
        team_id: "other".into(),
        run: EvalRun {
            id: "other-run".into(),
            ..run.run.clone()
        },
        ..run.clone()
    };
    assert_eq!(repository.create("key", &other).await.unwrap(), other);
    assert_eq!(repository.list().await.unwrap().len(), 2);
    assert!(matches!(
        repository.create("new-key", &run).await,
        Err(RunError::Conflict)
    ));
}

#[rstest]
#[tokio::test]
async fn concurrent_idempotent_creation_publishes_exactly_one_run(
    #[future(awt)] database: Database,
    run: StoredRun,
) {
    let created = create_concurrently(&database, &run).await;
    assert_eq!(created.len(), 16);
    assert_eq!(
        created
            .iter()
            .map(|run| &run.run.id)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        1
    );
    assert_eq!(database.repository().list().await.unwrap().len(), 1);
}

#[rstest]
#[case::run("eval-run/%")]
#[case::idempotency("eval-idempotency/%")]
#[tokio::test]
async fn failed_creation_never_publishes_partial_run_or_idempotency(
    #[future(awt)] database: Database,
    run: StoredRun,
    #[case] rejected: &str,
) {
    database.sql(&format!("ALTER TABLE lens_state_blobs ADD CONSTRAINT reject_eval CHECK key NOT LIKE '{rejected}'")).await;
    assert!(matches!(
        database.repository().create("key", &run).await,
        Err(RunError::Unavailable(_))
    ));
    assert!(
        database
            .repository()
            .get(&run.run.id)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        database
            .store
            .read(&record_key("eval-idempotency", &[&run.team_id, "key"]))
            .await
            .unwrap()
            .value
            .is_null()
    );
}

#[rstest]
#[tokio::test]
async fn concurrent_submissions_preserve_every_slot_and_replacements_keep_first_receipt(
    #[future(awt)] database: Database,
    run: StoredRun,
    now: DateTime<Utc>,
) {
    let repository = database.repository();
    repository.create("key", &run).await.unwrap();
    submit_concurrently(&database, &run).await;
    let current = repository.get(&run.run.id).await.unwrap().unwrap();
    assert_eq!(current.run.received_trials, 4);
    assert_eq!(
        current
            .submissions
            .iter()
            .map(|s| (s.case_id.as_str(), s.trial))
            .collect::<Vec<_>>(),
        vec![("case-a", 0), ("case-a", 1), ("case-b", 0), ("case-b", 1)]
    );
    let changed: CaseResult =
        serde_json::from_value(json!({"error":{"type":"Failure","message":"changed"}})).unwrap();
    repository
        .submit(&run.run.id, "case-a", 0, &changed, now + Duration::hours(1))
        .await
        .unwrap();
    let current = repository.get(&run.run.id).await.unwrap().unwrap();
    assert_eq!(current.run.received_trials, 4);
    assert_eq!(current.submissions[0].received_at, now);
    assert_eq!(current.submissions[0].result, changed);
    let before = database
        .store
        .read(&record_key("eval-run", &[&run.run.id]))
        .await
        .unwrap();
    repository
        .submit(&run.run.id, "case-a", 0, &changed, now + Duration::hours(2))
        .await
        .unwrap();
    assert_eq!(
        database
            .store
            .read(&record_key("eval-run", &[&run.run.id]))
            .await
            .unwrap(),
        before
    );
}

#[rstest]
#[case::unknown_case("missing", 0, false)]
#[case::invalid_trial("case-a", 2, false)]
#[case::invalid_result("case-a", 0, true)]
#[tokio::test]
async fn rejected_submissions_do_not_change_counts(
    #[future(awt)] database: Database,
    run: StoredRun,
    result: CaseResult,
    now: DateTime<Utc>,
    #[case] case: &str,
    #[case] trial: u32,
    #[case] invalid: bool,
) {
    let repository = database.repository();
    repository.create("key", &run).await.unwrap();
    let result = repository
        .submit(
            &run.run.id,
            case,
            trial,
            &if invalid {
                CaseResult::default()
            } else {
                result
            },
            now,
        )
        .await;
    assert!(matches!(
        (case, trial, invalid, result),
        ("missing", _, _, Err(RunError::UnknownCase))
            | (_, 2, _, Err(RunError::InvalidTrial))
            | (_, _, true, Err(RunError::InvalidResult))
    ));
    assert_eq!(repository.get(&run.run.id).await.unwrap().unwrap(), run);
}

#[rstest]
#[tokio::test]
async fn finishing_closes_submission_and_allows_one_scoring_owner(
    #[future(awt)] database: Database,
    run: StoredRun,
    result: CaseResult,
    now: DateTime<Utc>,
) {
    let repository = database.repository();
    repository.create("key", &run).await.unwrap();
    assert!(
        repository
            .claim(&run.run.id, "worker", now, now + Duration::minutes(5))
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        repository.finish(&run.run.id).await.unwrap().status,
        RunStatus::Scoring
    );
    assert!(matches!(
        repository.finish(&run.run.id).await,
        Err(RunError::Closed)
    ));
    assert!(matches!(
        repository
            .submit(&run.run.id, "case-a", 0, &result, now)
            .await,
        Err(RunError::Closed)
    ));
    let claims = claim_concurrently(&database, &run.run.id).await;
    assert_eq!(claims.iter().filter(|claim| claim.is_some()).count(), 1);
    assert!(
        repository
            .get(&run.run.id)
            .await
            .unwrap()
            .unwrap()
            .lease
            .is_some()
    );
}

#[rstest]
#[case::before_expiry(-1,false)]
#[case::at_expiry(0, true)]
#[case::after_expiry(1, true)]
#[tokio::test]
async fn scoring_claims_recover_at_expiry(
    #[future(awt)] database: Database,
    run: StoredRun,
    now: DateTime<Utc>,
    #[case] offset: i64,
    #[case] allowed: bool,
) {
    let assigned = scoring(&database, &run).await;
    let next = now + Duration::minutes(5) + Duration::microseconds(offset);
    let reclaimed = database
        .repository()
        .claim(&run.run.id, "successor", next, next + Duration::minutes(5))
        .await
        .unwrap();
    assert_eq!(reclaimed.is_some(), allowed);
    if let Some(reclaimed) = reclaimed {
        assert_eq!(reclaimed.lease.unwrap().owner, "successor");
        assert!(reclaimed.version > assigned.version);
    }
}

#[rstest]
#[case::complete(0)]
#[case::fail(1)]
#[case::release(2)]
#[tokio::test]
async fn replaced_scoring_owner_cannot_publish_or_release_successor(
    #[future(awt)] database: Database,
    run: StoredRun,
    evaluation: Evaluation,
    now: DateTime<Utc>,
    #[case] operation: usize,
) {
    let assigned = scoring(&database, &run).await;
    let repository = database.repository();
    let next = now + Duration::minutes(5);
    let successor = repository
        .claim(&run.run.id, "successor", next, next + Duration::minutes(5))
        .await
        .unwrap()
        .unwrap();
    let result = match operation {
        0 => repository.complete(&assigned, &evaluation, next).await,
        1 => repository.fail(&assigned, "stale", next).await,
        _ => repository.release(&assigned).await,
    };
    assert!(matches!(result, Err(RunError::StaleLease)));
    assert_eq!(
        repository.get(&run.run.id).await.unwrap().unwrap(),
        successor
    );
}

#[rstest]
#[case::complete(false)]
#[case::fail(true)]
#[tokio::test]
async fn terminal_publication_clears_lease_and_keeps_summary_and_verdicts_together(
    #[future(awt)] database: Database,
    run: StoredRun,
    evaluation: Evaluation,
    now: DateTime<Utc>,
    #[case] failed: bool,
) {
    let assigned = scoring(&database, &run).await;
    let repository = database.repository();
    if failed {
        repository
            .fail(&assigned, "judge unavailable", now)
            .await
            .unwrap()
    } else {
        repository
            .complete(&assigned, &evaluation, now)
            .await
            .unwrap()
    }
    let completed = repository.get(&run.run.id).await.unwrap().unwrap();
    assert_eq!(
        completed.run.status,
        if failed {
            RunStatus::Failed
        } else {
            RunStatus::Done
        }
    );
    assert_eq!(
        completed.run.failure,
        if failed { "judge unavailable" } else { "" }
    );
    assert_eq!(completed.completed_at, Some(now));
    assert_eq!(completed.lease, None);
    assert_eq!(
        completed.run.summary,
        if failed {
            None
        } else {
            Some(evaluation.summary.clone())
        }
    );
    assert_eq!(
        completed.verdicts,
        if failed {
            Default::default()
        } else {
            evaluation.verdicts.clone()
        }
    );
    assert_eq!(
        completed.resolved_traces,
        if failed {
            Default::default()
        } else {
            evaluation.resolved_traces.clone()
        }
    );
    assert!(matches!(
        repository.complete(&assigned, &evaluation, now).await,
        Err(RunError::StaleLease)
    ));
    assert!(
        repository
            .claim(&run.run.id, "new", now, now + Duration::minutes(5))
            .await
            .unwrap()
            .is_none()
    );
}

#[rstest]
#[tokio::test]
async fn release_permits_retry_and_expired_owner_cannot_complete(
    #[future(awt)] database: Database,
    run: StoredRun,
    evaluation: Evaluation,
    now: DateTime<Utc>,
) {
    let assigned = scoring(&database, &run).await;
    let repository = database.repository();
    assert!(matches!(
        repository
            .complete(&assigned, &evaluation, now + Duration::minutes(5))
            .await,
        Err(RunError::StaleLease)
    ));
    repository.release(&assigned).await.unwrap();
    assert!(
        repository
            .get(&run.run.id)
            .await
            .unwrap()
            .unwrap()
            .lease
            .is_none()
    );
    let reclaimed = repository
        .claim(&run.run.id, "worker-2", now, now + Duration::minutes(1))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reclaimed.lease.unwrap().owner, "worker-2");
}

#[rstest]
#[tokio::test]
async fn renewal_extends_ownership_without_losing_frozen_inputs(
    #[future(awt)] database: Database,
    run: StoredRun,
    evaluation: Evaluation,
    now: DateTime<Utc>,
) {
    let assigned = scoring(&database, &run).await;
    let repository = database.repository();
    let renewed = repository
        .renew(
            &assigned,
            now + Duration::minutes(4),
            now + Duration::minutes(10),
        )
        .await
        .unwrap();
    assert_eq!(renewed.lease.as_ref().unwrap().owner, "worker");
    assert_eq!(
        renewed.lease.as_ref().unwrap().until,
        now + Duration::minutes(10)
    );
    assert_eq!(renewed.cases, assigned.cases);
    assert_eq!(renewed.submissions, assigned.submissions);
    assert!(renewed.version > assigned.version);
    assert!(matches!(
        repository.complete(&assigned, &evaluation, now).await,
        Err(RunError::StaleLease)
    ));
    let shortened = repository
        .renew(&renewed, now, now + Duration::minutes(1))
        .await
        .unwrap();
    assert_eq!(shortened, renewed);
    repository
        .complete(&renewed, &evaluation, now + Duration::minutes(9))
        .await
        .unwrap();
}

#[rstest]
#[case::exact_expiry(300, 600)]
#[case::expired(301, 600)]
#[case::new_lease_expired(0, 0)]
#[tokio::test]
async fn renewal_cannot_resurrect_expired_ownership(
    #[future(awt)] database: Database,
    run: StoredRun,
    now: DateTime<Utc>,
    #[case] elapsed: i64,
    #[case] until: i64,
) {
    let assigned = scoring(&database, &run).await;
    let result = database
        .repository()
        .renew(
            &assigned,
            now + Duration::seconds(elapsed),
            now + Duration::seconds(until),
        )
        .await;
    assert!(matches!(
        (until, result),
        (0, Err(RunError::InvalidRun)) | (_, Err(RunError::StaleLease))
    ));
    assert_eq!(
        database
            .repository()
            .get(&run.run.id)
            .await
            .unwrap()
            .unwrap(),
        assigned
    );
}

#[rstest]
#[case::version(0)]
#[case::owner(1)]
#[case::lease_deadline(2)]
#[case::missing_lease(3)]
#[tokio::test]
async fn completion_requires_the_exact_claim_snapshot(
    #[future(awt)] database: Database,
    run: StoredRun,
    evaluation: Evaluation,
    now: DateTime<Utc>,
    #[case] changed: usize,
) {
    let assigned = scoring(&database, &run).await;
    let expected = match changed {
        0 => StoredRun {
            version: assigned.version + 1,
            ..assigned.clone()
        },
        1 => StoredRun {
            lease: Some(lens_evals::ScoringLease {
                owner: "another".into(),
                ..assigned.lease.clone().unwrap()
            }),
            ..assigned.clone()
        },
        2 => StoredRun {
            lease: Some(lens_evals::ScoringLease {
                until: now + Duration::minutes(6),
                ..assigned.lease.clone().unwrap()
            }),
            ..assigned.clone()
        },
        _ => StoredRun {
            lease: None,
            ..assigned.clone()
        },
    };
    assert!(matches!(
        database
            .repository()
            .complete(&expected, &evaluation, now)
            .await,
        Err(RunError::StaleLease)
    ));
    assert_eq!(
        database
            .repository()
            .get(&run.run.id)
            .await
            .unwrap()
            .unwrap(),
        assigned
    );
}

#[rstest]
#[case::empty_owner("", 1)]
#[case::empty_lease("worker", 0)]
#[case::expired_lease("worker", -1)]
#[tokio::test]
async fn invalid_claims_do_not_create_ownership(
    #[future(awt)] database: Database,
    run: StoredRun,
    now: DateTime<Utc>,
    #[case] owner: &str,
    #[case] duration: i64,
) {
    let repository = database.repository();
    repository.create("key", &run).await.unwrap();
    repository.finish(&run.run.id).await.unwrap();
    assert!(matches!(
        repository
            .claim(&run.run.id, owner, now, now + Duration::seconds(duration))
            .await,
        Err(RunError::InvalidRun)
    ));
    assert!(
        repository
            .get(&run.run.id)
            .await
            .unwrap()
            .unwrap()
            .lease
            .is_none()
    );
}

#[rstest]
#[case::get(0)]
#[case::list(1)]
#[case::create(2)]
#[case::submit(3)]
#[case::finish(4)]
#[case::claim(5)]
#[case::complete(6)]
#[case::renew(7)]
#[case::fail(8)]
#[case::release(9)]
#[tokio::test]
async fn unavailable_storage_is_reported_for_every_eval_operation(
    #[future(awt)] database: Database,
    run: StoredRun,
    result: CaseResult,
    evaluation: Evaluation,
    now: DateTime<Utc>,
    #[case] operation: usize,
) {
    let assigned = scoring(&database, &run).await;
    database.sql("DROP TABLE lens_state_heads SYNC").await;
    let repository = database.repository();
    let until = now + Duration::minutes(10);
    let result = match operation {
        0 => repository.get(&run.run.id).await.map(|_| ()),
        1 => repository.list().await.map(|_| ()),
        2 => repository.create("new", &run).await.map(|_| ()),
        3 => {
            repository
                .submit(&run.run.id, "case-a", 0, &result, now)
                .await
        }
        4 => repository.finish(&run.run.id).await.map(|_| ()),
        5 => repository
            .claim(&run.run.id, "worker", now, until)
            .await
            .map(|_| ()),
        6 => repository.complete(&assigned, &evaluation, now).await,
        7 => repository.renew(&assigned, now, until).await.map(|_| ()),
        8 => repository.fail(&assigned, "failure", now).await,
        _ => repository.release(&assigned).await,
    };
    assert!(matches!(result, Err(RunError::Unavailable(_))));
}

#[rstest]
#[tokio::test]
async fn failed_result_publication_does_not_change_visible_submission(
    #[future(awt)] database: Database,
    run: StoredRun,
    result: CaseResult,
    now: DateTime<Utc>,
) {
    let repository = database.repository();
    repository.create("key", &run).await.unwrap();
    database.sql("ALTER TABLE lens_state_blobs ADD CONSTRAINT reject_update CHECK key NOT LIKE 'eval-run/%'").await;
    assert!(matches!(
        repository
            .submit(&run.run.id, "case-a", 0, &result, now)
            .await,
        Err(RunError::Unavailable(_))
    ));
    assert_eq!(repository.get(&run.run.id).await.unwrap().unwrap(), run);
}

#[rstest]
#[tokio::test]
async fn run_and_first_receipt_survive_abrupt_restart(
    #[future(awt)] isolated_database: Database,
    run: StoredRun,
    result: CaseResult,
    now: DateTime<Utc>,
) {
    let repository = isolated_database.repository();
    repository.create("key", &run).await.unwrap();
    repository
        .submit(&run.run.id, "case-a", 0, &result, now)
        .await
        .unwrap();
    repository.finish(&run.run.id).await.unwrap();
    let before = repository.get(&run.run.id).await.unwrap().unwrap();
    let restarted = litellm_storage_clickhouse::evals::Evals(isolated_database.restart().await);
    assert_eq!(restarted.get(&run.run.id).await.unwrap().unwrap(), before);
    assert_eq!(restarted.create("key", &run).await.unwrap(), before);
    assert_eq!(
        restarted
            .claim(&run.run.id, "restarted", now, now + Duration::minutes(5))
            .await
            .unwrap()
            .unwrap()
            .submissions[0]
            .received_at,
        now
    );
}

#[rstest]
#[tokio::test]
async fn unpublished_future_record_does_not_replace_the_run(
    #[future(awt)] database: Database,
    run: StoredRun,
) {
    let repository = database.repository();
    repository.create("key", &run).await.unwrap();
    let previous = database
        .store
        .read(&record_key("eval-run", &[&run.run.id]))
        .await
        .unwrap();
    database
        .store
        .prepare(vec![Change {
            previous,
            value: json!({"future":"unpublished"}),
        }])
        .await
        .unwrap();
    assert_eq!(repository.get(&run.run.id).await.unwrap().unwrap(), run);
    assert_eq!(repository.list().await.unwrap(), vec![run]);
}

#[rstest]
#[tokio::test]
async fn listing_pages_past_transport_limits_and_skips_tombstones(
    #[future(awt)] database: Database,
    run: StoredRun,
) {
    let runs = seed_runs(&database, &run).await;
    database
        .seed(&record_key("eval-run", &["deleted"]), json!(null))
        .await;
    database
        .seed("eval-run-unrelated/record", json!({"invalid":true}))
        .await;
    assert_eq!(database.repository().list().await.unwrap(), runs);
}

#[rstest]
#[case::get(false)]
#[case::list(true)]
#[tokio::test]
async fn malformed_run_records_fail_closed(
    #[future(awt)] database: Database,
    run: StoredRun,
    #[case] list: bool,
    #[values(false, true)] positional: bool,
) {
    let malformed = if positional {
        json!([
            run.run,
            run.spec,
            run.cases,
            run.team_id,
            run.version,
            run.created_at,
            run.completed_at,
            run.submissions,
            run.verdicts,
            run.resolved_traces,
            run.lease
        ])
    } else {
        json!({"invalid":true})
    };
    database
        .seed(&record_key("eval-run", &["bad"]), malformed)
        .await;
    let result = if list {
        database.repository().list().await.map(|_| ())
    } else {
        database.repository().get("bad").await.map(|_| ())
    };
    assert!(matches!(result, Err(RunError::Unavailable(_))));
}

#[rstest]
#[case::empty_key(true)]
#[case::empty_id(false)]
#[tokio::test]
async fn invalid_creation_leaves_no_records(
    #[future(awt)] database: Database,
    run: StoredRun,
    #[case] empty_key: bool,
) {
    let candidate = StoredRun {
        run: EvalRun {
            id: if empty_key {
                run.run.id.clone()
            } else {
                String::new()
            },
            ..run.run.clone()
        },
        ..run
    };
    assert!(matches!(
        database
            .repository()
            .create(if empty_key { "" } else { "key" }, &candidate)
            .await,
        Err(RunError::InvalidRun)
    ));
    assert!(database.repository().list().await.unwrap().is_empty());
}

#[rstest]
#[tokio::test]
async fn version_overflow_never_publishes_a_result(
    #[future(awt)] database: Database,
    run: StoredRun,
    result: CaseResult,
    now: DateTime<Utc>,
) {
    let run = StoredRun {
        version: u64::MAX,
        ..run
    };
    database
        .seed(
            &record_key("eval-run", &[&run.run.id]),
            serde_json::to_value(&run).unwrap(),
        )
        .await;
    assert!(matches!(
        database
            .repository()
            .submit(&run.run.id, "case-a", 0, &result, now)
            .await,
        Err(RunError::InvalidRun)
    ));
    assert_eq!(
        database
            .repository()
            .get(&run.run.id)
            .await
            .unwrap()
            .unwrap(),
        run
    );
}

#[rstest]
#[case::submit(0)]
#[case::finish(1)]
#[case::claim(2)]
#[case::complete(3)]
#[case::renew(4)]
#[case::fail(5)]
#[case::release(6)]
#[tokio::test]
async fn missing_runs_have_typed_failure_for_mutations(
    #[future(awt)] database: Database,
    run: StoredRun,
    result: CaseResult,
    evaluation: Evaluation,
    now: DateTime<Utc>,
    #[case] operation: usize,
) {
    let repository = database.repository();
    let until = now + Duration::minutes(10);
    let result = match operation {
        0 => {
            repository
                .submit(&run.run.id, "case-a", 0, &result, now)
                .await
        }
        1 => repository.finish(&run.run.id).await.map(|_| ()),
        2 => repository
            .claim(&run.run.id, "worker", now, until)
            .await
            .map(|_| ()),
        3 => repository.complete(&run, &evaluation, now).await,
        4 => repository.renew(&run, now, until).await.map(|_| ()),
        5 => repository.fail(&run, "failure", now).await,
        _ => repository.release(&run).await,
    };
    assert!(matches!(result, Err(RunError::NotFound)));
}
