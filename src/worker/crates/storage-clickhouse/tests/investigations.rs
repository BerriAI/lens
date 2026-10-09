#[path = "investigations/compaction.rs"]
mod compaction;
#[path = "investigations/finding_runs.rs"]
mod finding_runs;
#[path = "investigations/reviews.rs"]
mod reviews;
#[path = "investigations/scheduling.rs"]
mod scheduling;
#[path = "investigations/support.rs"]
mod support;
#[path = "investigations/trace_findings.rs"]
mod trace_findings;
#[path = "investigations/workers.rs"]
mod workers;

use chrono::{DateTime, Duration, Utc};
use lens_contract::{
    investigations::{Lens, Public, Scope},
    worker::{Job, JobStatus},
};
use lens_investigations::{LensRepository, RepositoryError, due_at};
use litellm_http::Client;
use litellm_storage_clickhouse::{
    Connection, Error,
    investigations::Investigations,
    state::{Change, ClickHouseState, Snapshot},
};
use rstest::rstest;
use serde_json::{Value, json};
use support::{
    Database, all, archived, concurrent_changes, database, isolated_database, job, jobs, lens, now,
    record_key, seed_lenses, stored, value,
};

#[rstest]
#[tokio::test]
async fn creation_and_noop_preserve_public_defaults_and_version(
    #[future(awt)] database: Database,
    lens: Lens,
) {
    let repository = database.repository();
    repository.initialize().await.unwrap();
    let created = repository.create(&lens).await.unwrap();
    assert_eq!(value(&created), value(&lens));
    let key = record_key("lens", &[&lens.id]);
    let before = database.store.read(&key).await.unwrap();
    assert_eq!(before.value, stored(&lens));
    assert!(before.value["lens"]["settings"].get("agent_name").is_some());
    assert_eq!(
        value(repository.replace(&lens, &lens).await.unwrap()),
        value(&lens)
    );
    assert_eq!(database.store.read(&key).await.unwrap(), before);
    assert_eq!(
        value(repository.get(&lens.id).await.unwrap().unwrap()),
        value(&lens)
    );
    assert_eq!(
        value(repository.lenses(&all()).await.unwrap()),
        value(vec![lens])
    );
}

#[rstest]
#[case::create(true)]
#[case::replace(false)]
#[tokio::test]
async fn concurrent_writers_have_one_winner(
    #[future(awt)] database: Database,
    lens: Lens,
    #[case] create: bool,
) {
    if !create {
        database.repository().create(&lens).await.unwrap();
    }
    let results = concurrent_changes(&database, &lens, create).await;
    let winners: Vec<_> = results
        .iter()
        .filter_map(|result| result.as_ref().ok())
        .collect();
    assert_eq!(winners.len(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(RepositoryError::Conflict)))
            .count(),
        15
    );
    assert_eq!(
        value(database.repository().get(&lens.id).await.unwrap().unwrap()),
        value(winners[0])
    );
    assert_eq!(winners[0].version, if create { 0 } else { 1 });
}

#[rstest]
#[tokio::test]
async fn missing_records_return_empty_reads_and_conflicting_replace(
    #[future(awt)] database: Database,
    lens: Lens,
) {
    let repository = database.repository();
    assert!(repository.get("missing").await.unwrap().is_none());
    assert!(repository.lenses(&all()).await.unwrap().is_empty());
    assert!(repository.jobs("missing", 0).await.unwrap().is_empty());
    assert!(repository.job("missing", "job").await.unwrap().is_none());
    assert!(matches!(
        repository.replace(&lens, &lens).await,
        Err(RepositoryError::Conflict)
    ));
}

#[rstest]
#[tokio::test]
async fn stale_version_and_renamed_candidate_cannot_overwrite_current(
    #[future(awt)] database: Database,
    lens: Lens,
) {
    let repository = database.repository();
    repository.create(&lens).await.unwrap();
    let candidate = Lens {
        spent: 2.0,
        version: 999,
        ..lens.clone()
    };
    let updated = repository.replace(&lens, &candidate).await.unwrap();
    assert_eq!(updated.version, lens.version + 1);
    assert_eq!(updated.spent, 2.0);
    assert!(matches!(
        repository.replace(&lens, &lens).await,
        Err(RepositoryError::Conflict)
    ));
    let renamed = Lens {
        id: "other".into(),
        ..updated.clone()
    };
    assert!(matches!(
        repository.replace(&updated, &renamed).await,
        Err(RepositoryError::Unavailable(_))
    ));
    assert_eq!(
        value(repository.get(&lens.id).await.unwrap().unwrap()),
        value(updated)
    );
    assert!(repository.get("other").await.unwrap().is_none());
}

#[rstest]
#[tokio::test]
async fn version_overflow_does_not_publish_a_partial_change(
    #[future(awt)] database: Database,
    lens: Lens,
) {
    let original = Lens {
        version: i64::MAX,
        ..lens
    };
    let repository = database.repository();
    repository.create(&original).await.unwrap();
    let candidate = Lens {
        spent: 5.0,
        ..original.clone()
    };
    assert!(matches!(
        repository.replace(&original, &candidate).await,
        Err(RepositoryError::Unavailable(_))
    ));
    assert_eq!(
        value(repository.get(&original.id).await.unwrap().unwrap()),
        value(original)
    );
}

#[rstest]
#[case::team(Scope { team_id: "alpha".into(), ..Scope::default() }, true)]
#[case::other_team(Scope { team_id: "beta".into(), ..Scope::default() }, false)]
#[case::all(all(), true)]
#[case::key(Scope { api_key_hash: "secret".into(), ..Scope::default() }, false)]
#[tokio::test]
async fn list_filters_scope_before_returning_records(
    #[future(awt)] database: Database,
    lens: Lens,
    #[case] scope: Scope,
    #[case] visible: bool,
) {
    database.repository().create(&lens).await.unwrap();
    let records = database.repository().lenses(&scope).await.unwrap();
    assert_eq!(records.len(), usize::from(visible));
    if visible {
        assert_eq!(value(&records[0]), value(lens));
    }
}

#[rstest]
#[case::own_key(Scope { api_key_hash: "alpha-key".into(), ..Scope::default() }, true)]
#[case::other_key(Scope { api_key_hash: "other".into(), ..Scope::default() }, false)]
#[case::unscoped(Scope::default(), false)]
#[case::all(all(), true)]
#[tokio::test]
async fn key_scoped_records_require_the_matching_key(
    #[future(awt)] database: Database,
    lens: Lens,
    #[case] scope: Scope,
    #[case] visible: bool,
) {
    let target = Lens {
        scope: Scope {
            api_key_hash: "alpha-key".into(),
            ..Scope::default()
        },
        ..lens
    };
    database.repository().create(&target).await.unwrap();
    assert_eq!(
        database.repository().lenses(&scope).await.unwrap().len(),
        usize::from(visible)
    );
}

#[rstest]
#[tokio::test]
async fn listing_pages_past_128_keys_and_ignores_tombstones(#[future(awt)] database: Database) {
    let lenses = seed_lenses(&database, 130).await;
    database
        .seed("lens/%5B%22tombstone%22%5D", Value::Null)
        .await;
    assert_eq!(
        value(database.repository().lenses(&all()).await.unwrap()),
        value(lenses)
    );
}

#[rstest]
#[case::idle(None, true)]
#[case::disabled(None, false)]
#[case::queued(Some(JobStatus::Queued), false)]
#[case::running(Some(JobStatus::Running), false)]
#[case::completed(Some(JobStatus::Completed), true)]
#[tokio::test]
async fn changed_lens_recomputes_due_time(
    #[future(awt)] database: Database,
    lens: Lens,
    job: Job,
    now: DateTime<Utc>,
    #[case] status: Option<JobStatus>,
    #[case] enabled: bool,
) {
    let repository = database.repository();
    repository.create(&lens).await.unwrap();
    let mut candidate = Lens {
        next_run_at: now + Duration::hours(1),
        ..lens.clone()
    };
    candidate.settings.enabled = enabled;
    candidate.jobs = status
        .map(|status| Job {
            status,
            lease_until: Some(now + Duration::minutes(5)),
            ..job
        })
        .into_iter()
        .collect();
    let updated = repository.replace(&lens, &candidate).await.unwrap();
    let snapshot = database
        .store
        .read(&record_key("lens", &[&lens.id]))
        .await
        .unwrap();
    assert_eq!(snapshot.value["due_at"], value(due_at(&updated)));
    assert_eq!(snapshot.value["lens"]["version"], lens.version + 1);
    let schedule = database.sql("SELECT id, toInt32(version) AS version, toString(toUnixTimestamp64Micro(due_at)) AS due_at FROM lens_schedule FINAL INNER JOIN lens_state_heads USING (key, revision, digest) FORMAT JSONEachRow").await;
    assert_eq!(
        serde_json::from_str::<Value>(&schedule).unwrap(),
        json!({
            "id": updated.id,
            "version": updated.version,
            "due_at": due_at(&updated).map(|at| at.timestamp_micros().to_string()),
        })
    );
}

#[rstest]
#[tokio::test]
async fn removed_jobs_and_parent_are_published_together(
    #[future(awt)] database: Database,
    lens: Lens,
    job: Job,
) {
    let original = Lens {
        jobs: vec![job.clone()],
        ..lens
    };
    let repository = database.repository();
    repository.create(&original).await.unwrap();
    let updated = repository
        .replace(
            &original,
            &Lens {
                jobs: vec![],
                ..original.clone()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        value(Public(repository.jobs(&original.id, 0).await.unwrap())),
        value(Public(vec![job.clone()]))
    );
    assert_eq!(
        value(Public(
            repository
                .job(&original.id, &job.id)
                .await
                .unwrap()
                .unwrap()
        )),
        value(Public(&job))
    );
    let archived_snapshot = database
        .store
        .read(&record_key("run", &[&original.id, &job.id]))
        .await
        .unwrap();
    assert_eq!(archived_snapshot.value, archived(&updated, &job));
    assert!(archived_snapshot.value["job"].get("error").is_some());
}

#[rstest]
#[tokio::test]
async fn retained_jobs_are_not_archived_and_existing_archives_remain_immutable(
    #[future(awt)] database: Database,
    lens: Lens,
    job: Job,
) {
    let original = Lens {
        jobs: vec![job.clone()],
        ..lens
    };
    let repository = database.repository();
    repository.create(&original).await.unwrap();
    let changed = repository
        .replace(
            &original,
            &Lens {
                spent: 2.0,
                ..original.clone()
            },
        )
        .await
        .unwrap();
    let key = record_key("run", &[&original.id, &job.id]);
    assert!(database.store.read(&key).await.unwrap().value.is_null());
    let historical = Job { cost: 0.5, ..job };
    database.seed(&key, archived(&changed, &historical)).await;
    let before = database.store.read(&key).await.unwrap();
    repository
        .replace(
            &changed,
            &Lens {
                jobs: vec![],
                ..changed.clone()
            },
        )
        .await
        .unwrap();
    assert_eq!(database.store.read(&key).await.unwrap(), before);
}

#[rstest]
#[tokio::test]
async fn history_paginates_archived_and_current_jobs_by_time_then_id(
    #[future(awt)] database: Database,
    lens: Lens,
) {
    let jobs = jobs(56);
    let original = Lens {
        jobs: jobs.clone(),
        ..lens
    };
    let repository = database.repository();
    repository.create(&original).await.unwrap();
    repository
        .replace(
            &original,
            &Lens {
                jobs: jobs[54..].to_vec(),
                ..original.clone()
            },
        )
        .await
        .unwrap();
    let first = repository.jobs(&original.id, 0).await.unwrap();
    let second = repository.jobs(&original.id, 50).await.unwrap();
    assert_eq!(first.len(), 50);
    assert_eq!(second.len(), 6);
    assert_eq!(
        value(Public(first.into_iter().chain(second).collect::<Vec<_>>())),
        value(Public(jobs.into_iter().rev().collect::<Vec<_>>()))
    );
    assert!(repository.jobs(&original.id, 100).await.unwrap().is_empty());
    assert!(
        repository
            .job(&original.id, "absent")
            .await
            .unwrap()
            .is_none()
    );
}

#[rstest]
#[tokio::test]
async fn history_orders_time_before_identifier(
    #[future(awt)] database: Database,
    lens: Lens,
    job: Job,
    now: DateTime<Utc>,
) {
    let recent = Job {
        id: "a".into(),
        created_at: now + Duration::seconds(1),
        ..job.clone()
    };
    let older = Job {
        id: "z".into(),
        ..job
    };
    let original = Lens {
        jobs: vec![older.clone(), recent.clone()],
        ..lens
    };
    database.repository().create(&original).await.unwrap();
    assert_eq!(
        value(Public(
            database.repository().jobs(&original.id, 0).await.unwrap()
        )),
        value(Public(vec![recent, older]))
    );
}

#[rstest]
#[tokio::test]
async fn unpublished_archive_cannot_expose_obsolete_job_contents(
    #[future(awt)] database: Database,
    lens: Lens,
    job: Job,
) {
    let original = Lens {
        jobs: vec![job.clone()],
        ..lens
    };
    let repository = database.repository();
    repository.create(&original).await.unwrap();
    let parent = database
        .store
        .read(&record_key("lens", &[&original.id]))
        .await
        .unwrap();
    let removed = Lens {
        version: original.version + 1,
        jobs: vec![],
        ..original.clone()
    };
    let prepared = database
        .store
        .prepare(vec![
            Change {
                previous: parent,
                value: stored(&removed),
            },
            Change {
                previous: Snapshot::empty(record_key("run", &[&original.id, &job.id])),
                value: archived(&removed, &job),
            },
        ])
        .await
        .unwrap();
    assert_eq!(
        value(Public(repository.jobs(&original.id, 0).await.unwrap())),
        value(Public(vec![job.clone()]))
    );
    let corrected = Job { cost: 2.0, ..job };
    let updated = repository
        .replace(
            &original,
            &Lens {
                jobs: vec![corrected.clone()],
                ..original.clone()
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        database.store.publish(&prepared).await,
        Err(Error::StateConflict)
    ));
    assert_eq!(
        value(Public(repository.jobs(&original.id, 0).await.unwrap())),
        value(Public(vec![corrected.clone()]))
    );
    repository
        .replace(
            &updated,
            &Lens {
                jobs: vec![],
                ..updated.clone()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        value(Public(
            repository
                .job(&original.id, &corrected.id)
                .await
                .unwrap()
                .unwrap()
        )),
        value(Public(corrected))
    );
}

#[rstest]
#[tokio::test]
async fn archives_newer_than_parent_snapshot_are_not_visible(
    #[future(awt)] database: Database,
    lens: Lens,
    job: Job,
) {
    let repository = database.repository();
    repository.create(&lens).await.unwrap();
    let future = Lens {
        version: lens.version + 1,
        ..lens.clone()
    };
    database
        .seed(
            &record_key("run", &[&lens.id, &job.id]),
            archived(&future, &job),
        )
        .await;
    assert!(repository.jobs(&lens.id, 0).await.unwrap().is_empty());
    assert!(repository.job(&lens.id, &job.id).await.unwrap().is_none());
}

#[rstest]
#[tokio::test]
async fn legacy_unicode_keys_and_timestamps_remain_readable(
    #[future(awt)] database: Database,
    lens: Lens,
    job: Job,
) {
    let original = Lens {
        id: "lens/λ \"x\"".into(),
        jobs: vec![Job {
            id: "job/% 雪".into(),
            ..job
        }],
        ..lens
    };
    let key = "lens/%5B%22lens%2F%CE%BB%20%5C%22x%5C%22%22%5D";
    let mut legacy = stored(&original);
    legacy["lens"]["created_at"] = json!("2026-03-01T07:00:00.123456-05:00");
    legacy["lens"]
        .as_object_mut()
        .unwrap()
        .remove("criteria_updated_at");
    database.seed(key, legacy).await;
    let repository = database.repository();
    let loaded = repository.get(&original.id).await.unwrap().unwrap();
    assert_eq!(value(&loaded), value(&original));
    let updated = repository
        .replace(
            &loaded,
            &Lens {
                jobs: vec![],
                ..loaded.clone()
            },
        )
        .await
        .unwrap();
    let archive_key = "run/%5B%22lens%2F%CE%BB%20%5C%22x%5C%22%22%2C%22job%2F%25%20%E9%9B%AA%22%5D";
    assert_eq!(
        database.store.read(archive_key).await.unwrap().value,
        archived(&updated, &original.jobs[0])
    );
    assert_eq!(
        value(Public(repository.jobs(&original.id, 0).await.unwrap())),
        value(Public(original.jobs))
    );
}

#[rstest]
#[tokio::test]
async fn restart_preserves_current_lens_and_archived_history(
    #[future(awt)] isolated_database: Database,
    lens: Lens,
    job: Job,
) {
    let original = Lens {
        jobs: vec![job.clone()],
        ..lens
    };
    let repository = isolated_database.repository();
    repository.create(&original).await.unwrap();
    let updated = repository
        .replace(
            &original,
            &Lens {
                jobs: vec![],
                ..original.clone()
            },
        )
        .await
        .unwrap();
    let restarted = isolated_database.restart().await;
    assert_eq!(
        value(restarted.get(&original.id).await.unwrap().unwrap()),
        value(&updated)
    );
    assert_eq!(
        value(Public(restarted.jobs(&original.id, 0).await.unwrap())),
        value(Public(vec![job]))
    );
}

#[rstest]
#[case::malformed_lens(json!({"lens": {"id": "lens"}, "due_at": null}))]
#[case::missing_due_time(json!({"lens": support::lens()}))]
#[case::positional_lens(json!([support::lens(), null]))]
#[tokio::test]
async fn malformed_lens_and_unavailable_storage_fail_closed(
    #[future(awt)] database: Database,
    lens: Lens,
    #[case] malformed: Value,
) {
    database
        .seed(&record_key("lens", &[&lens.id]), malformed)
        .await;
    assert!(matches!(
        database.repository().get(&lens.id).await,
        Err(RepositoryError::Unavailable(_))
    ));
    assert!(matches!(
        database.repository().lenses(&all()).await,
        Err(RepositoryError::Unavailable(_))
    ));
    let unavailable = Investigations(ClickHouseState::new(
        Client::no_redirect_for_test(),
        Connection::parse("http://127.0.0.1:9").unwrap(),
    ));
    assert!(matches!(
        unavailable.create(&lens).await,
        Err(RepositoryError::Unavailable(_))
    ));
    assert!(matches!(
        unavailable.get(&lens.id).await,
        Err(RepositoryError::Unavailable(_))
    ));
    assert!(matches!(
        unavailable.jobs(&lens.id, 0).await,
        Err(RepositoryError::Unavailable(_))
    ));
    assert!(matches!(
        unavailable.initialize().await,
        Err(RepositoryError::Unavailable(_))
    ));
}
