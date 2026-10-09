use super::*;
use lens_contract::investigations::Worker;
use lens_investigations::{ScheduleRepository, WorkerRepository};

#[rstest]
#[tokio::test]
async fn compaction_reclaims_indexes_without_changing_schedule_history_or_workers(
    #[future(awt)] database: Database,
    lens: Lens,
    job: Job,
    now: DateTime<Utc>,
) {
    let repository = database.repository();
    repository.initialize().await.unwrap();
    repository.create(&lens).await.unwrap();
    database
        .seed(
            &record_key("run", &[&lens.id, &job.id]),
            archived(&lens, &job),
        )
        .await;
    let worker = Worker {
        id: "compaction-worker".into(),
        name: "Retained worker".into(),
        scope: lens.scope.clone(),
        analysis_key_id: Some("a".repeat(64)),
        last_seen: now,
        revoked: false,
    };
    repository
        .save_worker(&worker, Some("retained-token-hash"))
        .await
        .unwrap();
    let before_due: Vec<_> = repository
        .due(&all(), &now, 20, None)
        .await
        .unwrap()
        .into_iter()
        .map(|row| (value(row.lens), row.due_at))
        .collect();
    let before_jobs = value(repository.jobs(&lens.id, 0).await.unwrap());
    let before_workers = value(repository.eligible_workers(&lens.scope, "").await.unwrap());
    assert!(!before_due.is_empty());
    assert!(!before_jobs.as_array().unwrap().is_empty());
    assert!(!before_workers.as_array().unwrap().is_empty());
    let keys = database.store.keys("", "", 100).await.unwrap();
    let references: Vec<_> = keys.iter().map(String::as_str).collect();
    database.store.compact(&references).await.unwrap();
    database.store.compact(&references).await.unwrap();
    assert_eq!(
        value(repository.get(&lens.id).await.unwrap().unwrap()),
        value(&lens)
    );
    assert_eq!(
        repository
            .due(&all(), &now, 20, None)
            .await
            .unwrap()
            .into_iter()
            .map(|row| (value(row.lens), row.due_at))
            .collect::<Vec<_>>(),
        before_due
    );
    assert_eq!(
        value(repository.jobs(&lens.id, 0).await.unwrap()),
        before_jobs
    );
    assert_eq!(
        value(repository.eligible_workers(&lens.scope, "").await.unwrap()),
        before_workers
    );
    assert_eq!(
        value(
            repository
                .worker("retained-token-hash")
                .await
                .unwrap()
                .unwrap()
        ),
        value(worker)
    );
    assert_eq!(database.sql("SELECT (SELECT count() FROM lens_schedule FINAL), (SELECT count() FROM lens_jobs FINAL), (SELECT count() FROM lens_workers FINAL)").await.trim(), "1\t1\t1");
    assert_eq!(
        database
            .sql("SELECT count() FROM lens_state_blobs FINAL")
            .await
            .trim(),
        keys.len().to_string()
    );
}
