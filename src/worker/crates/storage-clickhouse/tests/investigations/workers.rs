use chrono::{DateTime, Duration, Utc};
use lens_contract::investigations::{Scope, Worker};
use lens_investigations::{RepositoryError, WorkerRepository};
use litellm_storage_clickhouse::state::{Change, Snapshot};
use rstest::{fixture, rstest};
use serde_json::json;

use super::support::{Database, all, database, isolated_database, now, record_key, value};

const NEW_KEY: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const OTHER_KEY: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

#[fixture]
fn worker(now: DateTime<Utc>) -> Worker {
    Worker {
        id: "worker 雪/'\"".into(),
        name: "Review worker".into(),
        scope: Scope {
            team_id: "alpha".into(),
            ..Scope::default()
        },
        analysis_key_id: Some("a".repeat(64)),
        last_seen: now,
        revoked: false,
    }
}

#[rstest]
#[tokio::test]
async fn worker_token_and_record_roundtrip_with_legacy_keys(
    #[future(awt)] database: Database,
    worker: Worker,
    now: DateTime<Utc>,
) {
    let repository = database.repository();
    repository
        .save_worker(&worker, Some("digest/雪"))
        .await
        .unwrap();
    let key = record_key("worker", &[&worker.id]);
    assert_eq!(
        database.store.read(&key).await.unwrap().value,
        value(&worker)
    );
    assert_eq!(
        database
            .store
            .read(&record_key("worker-token", &["digest/雪"]))
            .await
            .unwrap()
            .value,
        json!(worker.id)
    );
    assert_eq!(
        value(repository.worker("digest/雪").await.unwrap().unwrap()),
        value(&worker)
    );
    assert!(repository.worker("missing").await.unwrap().is_none());
    assert_eq!(
        value(repository.workers().await.unwrap()),
        value(vec![worker.clone()])
    );
    let updated = Worker {
        name: "Updated".into(),
        ..worker
    };
    repository.save_worker(&updated, None).await.unwrap();
    assert_eq!(
        value(repository.worker("digest/雪").await.unwrap().unwrap()),
        value(&updated)
    );
    let previous = database.store.read(&key).await.unwrap();
    repository.save_worker(&updated, None).await.unwrap();
    repository.heartbeat(&updated.id, now).await.unwrap();
    assert_eq!(database.store.read(&key).await.unwrap(), previous);
}

#[rstest]
#[case::duplicate_token(true)]
#[case::duplicate_worker(false)]
#[tokio::test]
async fn duplicate_registration_cannot_publish_partial_records(
    #[future(awt)] database: Database,
    worker: Worker,
    #[case] duplicate_token: bool,
) {
    let repository = database.repository();
    repository
        .save_worker(&worker, Some("original"))
        .await
        .unwrap();
    let candidate = Worker {
        id: if duplicate_token {
            "other".into()
        } else {
            worker.id.clone()
        },
        ..worker.clone()
    };
    let token = if duplicate_token {
        "original"
    } else {
        "other-token"
    };
    assert!(matches!(
        repository.save_worker(&candidate, Some(token)).await,
        Err(RepositoryError::Conflict)
    ));
    assert_eq!(
        value(repository.workers().await.unwrap()),
        value(vec![worker.clone()])
    );
    assert!(repository.worker("other-token").await.unwrap().is_none());
    assert_eq!(
        value(repository.worker("original").await.unwrap().unwrap()),
        value(worker)
    );
}

#[rstest]
#[tokio::test]
async fn unknown_worker_updates_are_noops_and_orphan_tokens_return_none(
    #[future(awt)] database: Database,
    worker: Worker,
    now: DateTime<Utc>,
) {
    let repository = database.repository();
    repository.save_worker(&worker, None).await.unwrap();
    repository.heartbeat(&worker.id, now).await.unwrap();
    repository.revoke_worker(&worker.id).await.unwrap();
    assert!(
        repository
            .set_worker_billing(&worker.id, NEW_KEY)
            .await
            .unwrap()
            .is_none()
    );
    assert!(repository.workers().await.unwrap().is_empty());
    database
        .seed(&record_key("worker-token", &["orphan"]), json!(worker.id))
        .await;
    assert!(repository.worker("orphan").await.unwrap().is_none());
}

#[rstest]
#[tokio::test]
async fn billing_and_heartbeat_preserve_worker_fields_and_revocation(
    #[future(awt)] database: Database,
    worker: Worker,
    now: DateTime<Utc>,
) {
    let repository = database.repository();
    repository
        .save_worker(&worker, Some("token"))
        .await
        .unwrap();
    let updated = repository
        .set_worker_billing(&worker.id, NEW_KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.analysis_key_id.as_deref(), Some(NEW_KEY));
    assert_eq!(updated.name, worker.name);
    assert_eq!(updated.scope, worker.scope);
    let at = now + Duration::microseconds(1);
    repository.heartbeat(&worker.id, at).await.unwrap();
    repository.revoke_worker(&worker.id).await.unwrap();
    let revoked = repository.worker("token").await.unwrap().unwrap();
    assert!(revoked.revoked);
    assert_eq!(revoked.last_seen, at);
    assert_eq!(revoked.analysis_key_id, updated.analysis_key_id);
    let key = record_key("worker", &[&worker.id]);
    let before = database.store.read(&key).await.unwrap();
    assert!(
        repository
            .set_worker_billing(&worker.id, OTHER_KEY)
            .await
            .unwrap()
            .is_none()
    );
    repository.revoke_worker(&worker.id).await.unwrap();
    assert_eq!(database.store.read(&key).await.unwrap(), before);
    repository.heartbeat(&worker.id, now).await.unwrap();
    let heartbeat = repository.worker("token").await.unwrap().unwrap();
    assert!(heartbeat.revoked);
    assert_eq!(heartbeat.last_seen, now);
    assert_eq!(
        value(repository.workers().await.unwrap()),
        value(vec![heartbeat])
    );
    assert!(
        repository
            .eligible_workers(&worker.scope, "")
            .await
            .unwrap()
            .is_empty()
    );
}

async fn seed_scopes(database: &Database, worker: &Worker) {
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
            .save_worker(
                &Worker {
                    id: id.into(),
                    scope,
                    ..worker.clone()
                },
                Some(id),
            )
            .await
            .unwrap();
    }
}

#[rstest]
#[case::all_teams(all(), vec!["all"])]
#[case::team(Scope { team_id: "alpha".into(), ..Scope::default() }, vec!["all", "alpha"])]
#[case::key(Scope { api_key_hash: "secret".into(), ..Scope::default() }, vec!["all", "key"])]
#[case::other_key(Scope { api_key_hash: "other".into(), ..Scope::default() }, vec!["all"])]
#[case::empty(Scope::default(), vec!["all", "empty"])]
#[tokio::test]
async fn eligible_workers_can_cover_requested_scope(
    #[future(awt)] database: Database,
    worker: Worker,
    #[case] scope: Scope,
    #[case] expected: Vec<&str>,
) {
    seed_scopes(&database, &worker).await;
    let workers = database
        .repository()
        .eligible_workers(&scope, "")
        .await
        .unwrap();
    assert_eq!(
        workers
            .into_iter()
            .map(|worker| worker.id)
            .collect::<Vec<_>>(),
        expected
    );
}

async fn seed_workers(database: &Database, worker: &Worker, count: usize) -> Vec<Worker> {
    let workers: Vec<_> = (0..count)
        .map(|index| Worker {
            id: format!("worker-{index:03}"),
            ..worker.clone()
        })
        .collect();
    let changes = workers
        .iter()
        .map(|worker| Change {
            previous: Snapshot::empty(record_key("worker", &[&worker.id])),
            value: value(worker),
        })
        .collect();
    database.store.commit(changes).await.unwrap();
    workers
}

#[rstest]
#[tokio::test]
async fn worker_listing_and_eligibility_page_past_limits_and_ignore_obsolete_rows(
    #[future(awt)] database: Database,
    worker: Worker,
) {
    let entries = seed_workers(&database, &worker, 131).await;
    let repository = database.repository();
    let removed = database
        .store
        .read(&record_key("worker", &[&entries[130].id]))
        .await
        .unwrap();
    database
        .store
        .commit(vec![Change {
            previous: removed,
            value: json!(null),
        }])
        .await
        .unwrap();
    assert_eq!(
        value(repository.workers().await.unwrap()),
        value(&entries[..130])
    );
    let first = repository
        .eligible_workers(&worker.scope, "")
        .await
        .unwrap();
    let second = repository
        .eligible_workers(&worker.scope, &first.last().unwrap().id)
        .await
        .unwrap();
    let third = repository
        .eligible_workers(&worker.scope, &second.last().unwrap().id)
        .await
        .unwrap();
    assert_eq!(first.len(), 50);
    assert_eq!(second.len(), 50);
    assert_eq!(third.len(), 30);
    assert_eq!(
        value(
            first
                .into_iter()
                .chain(second)
                .chain(third)
                .collect::<Vec<_>>()
        ),
        value(&entries[..130])
    );
    assert!(
        repository
            .eligible_workers(&worker.scope, &entries[129].id)
            .await
            .unwrap()
            .is_empty()
    );
    repository.revoke_worker(&entries[0].id).await.unwrap();
    assert_eq!(
        repository
            .eligible_workers(&worker.scope, "")
            .await
            .unwrap()[0]
            .id,
        entries[1].id
    );
    let published = database
        .store
        .read(&record_key("worker", &[&entries[0].id]))
        .await
        .unwrap();
    let _prepared = database
        .store
        .prepare(vec![Change {
            previous: published,
            value: value(&entries[0]),
        }])
        .await
        .unwrap();
    assert_eq!(
        repository
            .eligible_workers(&worker.scope, "")
            .await
            .unwrap()[0]
            .id,
        entries[1].id
    );
}

#[rstest]
#[tokio::test]
async fn managed_registration_reuses_token_identity_and_can_restore_revoked_worker(
    #[future(awt)] database: Database,
    worker: Worker,
) {
    let repository = database.repository();
    let created = repository
        .configure_service_worker(&worker, "managed")
        .await
        .unwrap();
    assert_eq!(value(&created), value(&worker));
    repository.revoke_worker(&created.id).await.unwrap();
    let candidate = Worker {
        id: "new-candidate-id".into(),
        name: "New configuration".into(),
        scope: all(),
        ..worker.clone()
    };
    let configured = repository
        .configure_service_worker(&candidate, "managed")
        .await
        .unwrap();
    assert_eq!(configured.id, worker.id);
    assert_eq!(configured.name, candidate.name);
    assert_eq!(configured.scope, candidate.scope);
    assert!(!configured.revoked);
    assert_eq!(
        value(repository.worker("managed").await.unwrap().unwrap()),
        value(&configured)
    );
    assert_eq!(
        value(repository.workers().await.unwrap()),
        value(vec![configured])
    );
}

#[rstest]
#[tokio::test]
async fn managed_registration_does_not_steal_existing_id_for_new_token(
    #[future(awt)] database: Database,
    worker: Worker,
) {
    let repository = database.repository();
    repository
        .save_worker(&worker, Some("original"))
        .await
        .unwrap();
    assert!(matches!(
        repository
            .configure_service_worker(&worker, "new-token")
            .await,
        Err(RepositoryError::Conflict)
    ));
    assert!(repository.worker("new-token").await.unwrap().is_none());
    assert_eq!(
        value(repository.worker("original").await.unwrap().unwrap()),
        value(worker)
    );
}

async fn concurrent_registrations(
    database: &Database,
    worker: &Worker,
    managed: bool,
) -> Vec<Result<Worker, RepositoryError>> {
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(16));
    let mut tasks = tokio::task::JoinSet::new();
    for index in 0..16 {
        let repository = database.repository();
        let barrier = barrier.clone();
        let worker = Worker {
            id: format!("candidate-{index}"),
            name: format!("Worker {index}"),
            ..worker.clone()
        };
        tasks.spawn(async move {
            barrier.wait().await;
            if managed {
                repository
                    .configure_service_worker(&worker, "contended")
                    .await
            } else {
                repository
                    .save_worker(&worker, Some("contended"))
                    .await
                    .map(|()| worker)
            }
        });
    }
    let mut results = Vec::new();
    while let Some(result) = tasks.join_next().await {
        results.push(result.unwrap());
    }
    results
}

#[rstest]
#[case::external(false, 1)]
#[case::managed(true, 16)]
#[tokio::test]
async fn concurrent_registration_has_one_durable_token_owner(
    #[future(awt)] database: Database,
    worker: Worker,
    #[case] managed: bool,
    #[case] successes: usize,
) {
    let results = concurrent_registrations(&database, &worker, managed).await;
    assert_eq!(
        results.iter().filter(|result| result.is_ok()).count(),
        successes
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(RepositoryError::Conflict)))
            .count(),
        16 - successes
    );
    let stored = database.repository().workers().await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(
        value(
            database
                .repository()
                .worker("contended")
                .await
                .unwrap()
                .unwrap()
        ),
        value(&stored[0])
    );
    let successful_ids: std::collections::BTreeSet<_> = results
        .into_iter()
        .filter_map(Result::ok)
        .map(|worker| worker.id)
        .collect();
    assert_eq!(
        successful_ids,
        std::collections::BTreeSet::from([stored[0].id.clone()])
    );
}

#[rstest]
#[tokio::test]
async fn simultaneous_billing_and_heartbeat_keep_both_changes(
    #[future(awt)] database: Database,
    worker: Worker,
    now: DateTime<Utc>,
) {
    let repository = database.repository();
    repository
        .save_worker(&worker, Some("token"))
        .await
        .unwrap();
    let at = now + Duration::hours(1);
    let (billing, heartbeat) = tokio::join!(
        repository.set_worker_billing(&worker.id, NEW_KEY),
        repository.heartbeat(&worker.id, at)
    );
    billing.unwrap().unwrap();
    heartbeat.unwrap();
    let stored = repository.worker("token").await.unwrap().unwrap();
    assert_eq!(stored.analysis_key_id.as_deref(), Some(NEW_KEY));
    assert_eq!(stored.last_seen, at);
    let (billing, revocation) = tokio::join!(
        repository.set_worker_billing(&worker.id, OTHER_KEY),
        repository.revoke_worker(&worker.id)
    );
    billing.unwrap();
    revocation.unwrap();
    assert!(repository.worker("token").await.unwrap().unwrap().revoked);
    assert!(
        repository
            .set_worker_billing(&worker.id, OTHER_KEY)
            .await
            .unwrap()
            .is_none()
    );
}

#[rstest]
#[tokio::test]
async fn worker_and_schedule_survive_abrupt_database_restart(
    #[future(awt)] isolated_database: Database,
    worker: Worker,
    now: DateTime<Utc>,
) {
    use lens_investigations::{LensRepository, ScheduleRepository};
    let repository = isolated_database.repository();
    repository
        .save_worker(&worker, Some("restart"))
        .await
        .unwrap();
    let lens = super::support::lens();
    repository.create(&lens).await.unwrap();
    let reopened = isolated_database.restart().await;
    assert_eq!(
        value(reopened.worker("restart").await.unwrap().unwrap()),
        value(worker)
    );
    let due = reopened.due(&all(), &now, 20, None).await.unwrap();
    assert_eq!(value(&due[0].lens), value(lens));
}

#[rstest]
#[case::invalid_token(json!(12), None)]
#[case::invalid_worker(json!("broken"), Some(json!({"id": "broken"}))) ]
#[case::invalid_billing(json!("broken"), Some(json!({"id": "broken", "name": "Worker", "scope": {}, "last_seen": now(), "analysis_key_id": "invalid"}))) ]
#[tokio::test]
async fn malformed_worker_credentials_fail_closed(
    #[future(awt)] database: Database,
    #[case] token: serde_json::Value,
    #[case] record: Option<serde_json::Value>,
) {
    database
        .seed(&record_key("worker-token", &["malformed"]), token)
        .await;
    if let Some(record) = record {
        database
            .seed(&record_key("worker", &["broken"]), record)
            .await;
    }
    assert!(matches!(
        database.repository().worker("malformed").await,
        Err(RepositoryError::Unavailable(_))
    ));
}

enum WorkerAction {
    List,
    Eligible,
    Lookup,
    Create,
    Replace,
    Configure,
    Billing,
    Revoke,
    Heartbeat,
}

#[rstest]
#[case::list(WorkerAction::List)]
#[case::eligible(WorkerAction::Eligible)]
#[case::lookup(WorkerAction::Lookup)]
#[case::create(WorkerAction::Create)]
#[case::replace(WorkerAction::Replace)]
#[case::configure(WorkerAction::Configure)]
#[case::billing(WorkerAction::Billing)]
#[case::revoke(WorkerAction::Revoke)]
#[case::heartbeat(WorkerAction::Heartbeat)]
#[tokio::test]
async fn unavailable_worker_storage_reports_failure(
    #[future(awt)] database: Database,
    worker: Worker,
    now: DateTime<Utc>,
    #[case] action: WorkerAction,
) {
    let repository = database.repository();
    repository
        .save_worker(&worker, Some("token"))
        .await
        .unwrap();
    database.sql("DROP TABLE lens_state_heads SYNC").await;
    let result = match action {
        WorkerAction::List => repository.workers().await.map(|_| ()),
        WorkerAction::Eligible => repository
            .eligible_workers(&worker.scope, "")
            .await
            .map(|_| ()),
        WorkerAction::Lookup => repository.worker("token").await.map(|_| ()),
        WorkerAction::Create => repository.save_worker(&worker, Some("token")).await,
        WorkerAction::Replace => repository.save_worker(&worker, None).await,
        WorkerAction::Configure => repository
            .configure_service_worker(&worker, "token")
            .await
            .map(|_| ()),
        WorkerAction::Billing => repository
            .set_worker_billing(&worker.id, NEW_KEY)
            .await
            .map(|_| ()),
        WorkerAction::Revoke => repository.revoke_worker(&worker.id).await,
        WorkerAction::Heartbeat => repository.heartbeat(&worker.id, now).await,
    };
    assert!(matches!(result, Err(RepositoryError::Unavailable(_))));
}
