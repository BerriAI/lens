use chrono::{DateTime, Duration, Utc};
use lens_contract::investigations::{Lens, Scope};
use lens_investigations::{LensRepository, RepositoryError, ScheduleRepository};
use litellm_storage_clickhouse::state::Change;
use rstest::rstest;
use serde_json::json;

use super::support::{Database, all, database, lens, now, record_key, seed_lenses, stored, value};

async fn seed_scopes(database: &Database, lens: &Lens) {
    for (id, scope) in [
        ("all", all()),
        (
            "alpha",
            Scope {
                team_id: "alpha".into(),
                ..Scope::default()
            },
        ),
        (
            "beta",
            Scope {
                team_id: "beta".into(),
                ..Scope::default()
            },
        ),
        (
            "key",
            Scope {
                api_key_hash: "secret".into(),
                ..Scope::default()
            },
        ),
        ("empty", Scope::default()),
    ] {
        database
            .repository()
            .create(&Lens {
                id: id.into(),
                scope,
                ..lens.clone()
            })
            .await
            .unwrap();
    }
}

#[rstest]
#[case::all_teams(all(), vec!["all", "alpha", "beta", "empty", "key"])]
#[case::team(Scope { team_id: "alpha".into(), ..Scope::default() }, vec!["alpha"])]
#[case::key(Scope { api_key_hash: "secret".into(), ..Scope::default() }, vec!["key"])]
#[case::different_key(Scope { api_key_hash: "other".into(), ..Scope::default() }, vec![])]
#[case::empty(Scope::default(), vec!["empty"])]
#[tokio::test]
async fn due_candidates_enforce_scope(
    #[future(awt)] database: Database,
    lens: Lens,
    now: DateTime<Utc>,
    #[case] scope: Scope,
    #[case] expected: Vec<&str>,
) {
    seed_scopes(&database, &lens).await;
    let candidates = database
        .repository()
        .due(&scope, &now, 20, None)
        .await
        .unwrap();
    assert_eq!(
        candidates
            .into_iter()
            .map(|candidate| candidate.lens.id)
            .collect::<Vec<_>>(),
        expected
    );
}

#[rstest]
#[case::before(-1, false)]
#[case::equal(0, true)]
#[case::after(1, true)]
#[tokio::test]
async fn due_time_is_inclusive_to_microsecond_precision(
    #[future(awt)] database: Database,
    lens: Lens,
    now: DateTime<Utc>,
    #[case] micros: i64,
    #[case] visible: bool,
) {
    let repository = database.repository();
    repository.create(&lens).await.unwrap();
    let candidates = repository
        .due(&all(), &(now + Duration::microseconds(micros)), 20, None)
        .await
        .unwrap();
    assert_eq!(candidates.len(), usize::from(visible));
    if visible {
        assert_eq!(candidates[0].due_at, now);
        assert_eq!(value(&candidates[0].lens), value(&lens));
    }
}

#[rstest]
#[tokio::test]
async fn scheduler_keyset_pages_use_time_then_id_and_caller_limit(
    #[future(awt)] database: Database,
    lens: Lens,
    now: DateTime<Utc>,
) {
    let entries = seed_lenses(&database, 60).await;
    let older = Lens {
        id: "z-older".into(),
        next_run_at: now - Duration::seconds(1),
        ..lens
    };
    let repository = database.repository();
    repository.create(&older).await.unwrap();
    let first = repository.due(&all(), &now, 31, None).await.unwrap();
    let second = repository
        .due(&all(), &now, 31, first.last())
        .await
        .unwrap();
    assert_eq!(first.len(), 31);
    assert_eq!(second.len(), 30);
    assert_eq!(
        value(
            first
                .iter()
                .chain(&second)
                .map(|row| row.lens.clone())
                .collect::<Vec<_>>()
        ),
        value(std::iter::once(older).chain(entries).collect::<Vec<_>>())
    );
    assert!(
        repository
            .due(&all(), &now, 31, second.last())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        repository
            .due(&all(), &now, 0, None)
            .await
            .unwrap()
            .is_empty()
    );
}

#[rstest]
#[tokio::test]
async fn scheduler_ignores_unpublished_and_obsolete_metadata(
    #[future(awt)] database: Database,
    lens: Lens,
    now: DateTime<Utc>,
) {
    let disabled = Lens {
        settings: lens_contract::worker::LensSettings {
            enabled: false,
            ..lens.settings.clone()
        },
        ..lens.clone()
    };
    let repository = database.repository();
    repository.create(&disabled).await.unwrap();
    let previous = database
        .store
        .read(&record_key("lens", &[&lens.id]))
        .await
        .unwrap();
    let prepared = database
        .store
        .prepare(vec![Change {
            previous,
            value: stored(&lens),
        }])
        .await
        .unwrap();
    assert!(
        repository
            .due(&all(), &now, 20, None)
            .await
            .unwrap()
            .is_empty()
    );
    database.store.publish(&prepared).await.unwrap();
    assert_eq!(
        repository.due(&all(), &now, 20, None).await.unwrap().len(),
        1
    );
    repository.replace(&lens, &disabled).await.unwrap();
    assert!(
        repository
            .due(&all(), &now, 20, None)
            .await
            .unwrap()
            .is_empty()
    );
}

#[rstest]
#[case::matching_version(0, true)]
#[case::stale_version(1, false)]
#[tokio::test]
async fn schedule_repair_preserves_current_lens_and_requires_matching_version(
    #[future(awt)] database: Database,
    lens: Lens,
    now: DateTime<Utc>,
    #[case] version_offset: i64,
    #[case] repairs: bool,
) {
    let key = record_key("lens", &[&lens.id]);
    database
        .seed(&key, json!({"lens": lens, "due_at": null}))
        .await;
    let previous = database.store.read(&key).await.unwrap();
    let candidate = Lens {
        version: lens.version + version_offset,
        next_run_at: now + Duration::hours(2),
        ..lens.clone()
    };
    database.repository().sync_due(&candidate).await.unwrap();
    let current = database.store.read(&key).await.unwrap();
    assert_eq!(current.value["lens"], value(&lens));
    if repairs {
        assert_eq!(current.value, stored(&lens));
        assert_eq!(current.head.revision, previous.head.revision + 1);
    } else {
        assert_eq!(current, previous);
    }
}

#[rstest]
#[tokio::test]
async fn missing_or_correct_schedule_repair_does_not_publish(
    #[future(awt)] database: Database,
    lens: Lens,
) {
    let repository = database.repository();
    let key = record_key("lens", &[&lens.id]);
    repository.sync_due(&lens).await.unwrap();
    assert!(database.store.read(&key).await.unwrap().value.is_null());
    repository.create(&lens).await.unwrap();
    let previous = database.store.read(&key).await.unwrap();
    repository.sync_due(&lens).await.unwrap();
    assert_eq!(database.store.read(&key).await.unwrap(), previous);
}

#[rstest]
#[tokio::test]
async fn malformed_schedule_record_fails_closed(
    #[future(awt)] database: Database,
    lens: Lens,
    now: DateTime<Utc>,
) {
    database
        .seed(
            &record_key("lens", &[&lens.id]),
            json!({"lens": {"id": lens.id}, "due_at": now}),
        )
        .await;
    let repository = database.repository();
    assert!(matches!(
        repository.sync_due(&lens).await,
        Err(RepositoryError::Unavailable(_))
    ));
    assert!(matches!(
        repository.due(&all(), &now, 20, None).await,
        Err(RepositoryError::Unavailable(_))
    ));
}

#[rstest]
#[tokio::test]
async fn equivalent_utc_offsets_do_not_trigger_schedule_repair(
    #[future(awt)] database: Database,
    lens: Lens,
) {
    let key = record_key("lens", &[&lens.id]);
    database
        .seed(
            &key,
            json!({"lens": lens, "due_at": lens.next_run_at.to_rfc3339()}),
        )
        .await;
    let previous = database.store.read(&key).await.unwrap();
    database.repository().sync_due(&lens).await.unwrap();
    assert_eq!(database.store.read(&key).await.unwrap(), previous);
}

async fn repair_concurrently(database: &Database, lens: &Lens) -> Vec<Result<(), RepositoryError>> {
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(16));
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..16 {
        let repository = database.repository();
        let lens = lens.clone();
        let barrier = barrier.clone();
        tasks.spawn(async move {
            barrier.wait().await;
            repository.sync_due(&lens).await
        });
    }
    let mut results = Vec::new();
    while let Some(result) = tasks.join_next().await {
        results.push(result.unwrap());
    }
    results
}

#[rstest]
#[tokio::test]
async fn concurrent_schedule_repairs_ignore_lost_compare_and_swap(
    #[future(awt)] database: Database,
    lens: Lens,
) {
    let key = record_key("lens", &[&lens.id]);
    database
        .seed(&key, json!({"lens": lens, "due_at": null}))
        .await;
    let previous = database.store.read(&key).await.unwrap();
    let results = repair_concurrently(&database, &lens).await;
    assert_eq!(
        results
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .len(),
        16
    );
    let current = database.store.read(&key).await.unwrap();
    assert_eq!(current.head.revision, previous.head.revision + 1);
    assert_eq!(current.value, stored(&lens));
}

#[rstest]
#[tokio::test]
async fn unavailable_schedule_storage_reports_failure(
    #[future(awt)] database: Database,
    lens: Lens,
    now: DateTime<Utc>,
) {
    database.sql("DROP TABLE lens_state_heads SYNC").await;
    assert!(matches!(
        database.repository().sync_due(&lens).await,
        Err(RepositoryError::Unavailable(_))
    ));
    assert!(matches!(
        database.repository().due(&all(), &now, 20, None).await,
        Err(RepositoryError::Unavailable(_))
    ));
}
