use lens_contract::{
    investigations::Lens,
    worker::{Finding, Job},
};
use lens_investigations::{LensRepository, RepositoryError};
use litellm_storage_clickhouse::state::{Change, Snapshot};
use rstest::{fixture, rstest};
use serde_json::{Value, json};

use super::support::{Database, archived, database, job, lens, record_key, stored, value};

#[fixture]
fn finding() -> Finding {
    let recorded: Value = serde_json::from_str(include_str!(
        "../../../contract/tests/fixtures/investigations_public.json"
    ))
    .unwrap();
    serde_json::from_value(recorded["finding"].clone()).unwrap()
}

#[rstest]
#[tokio::test]
async fn finding_runs_include_distinct_current_and_archived_matches(
    #[future(awt)] database: Database,
    lens: Lens,
    job: Job,
    finding: Finding,
) {
    let other = Finding {
        id: "other".into(),
        ..finding.clone()
    };
    let older = Job {
        id: "a".into(),
        findings: Some(vec![finding.clone(), finding.clone()]),
        ..job.clone()
    };
    let newer = Job {
        id: "b".into(),
        findings: Some(vec![other, finding.clone()]),
        ..job
    };
    let original = Lens {
        jobs: vec![older, newer.clone()],
        ..lens
    };
    let repository = database.repository();
    repository.create(&original).await.unwrap();
    repository
        .replace(
            &original,
            &Lens {
                jobs: vec![newer],
                ..original.clone()
            },
        )
        .await
        .unwrap();
    repository
        .create(&Lens {
            id: "other-lens".into(),
            ..original.clone()
        })
        .await
        .unwrap();
    let runs = repository
        .finding_runs(
            &original.id,
            &["other".into(), finding.id.clone(), "absent".into()],
        )
        .await
        .unwrap();
    assert_eq!(
        value(runs),
        json!([
            {"finding_id": finding.id, "job_id": "a"},
            {"finding_id": finding.id, "job_id": "b"},
            {"finding_id": "other", "job_id": "b"}
        ])
    );
    assert_eq!(
        value(
            repository
                .finding_runs(&original.id, &["other".into()])
                .await
                .unwrap()
        ),
        json!([{"finding_id":"other","job_id":"b"}])
    );
    assert!(
        repository
            .finding_runs("missing", &[finding.id])
            .await
            .unwrap()
            .is_empty()
    );
}

#[rstest]
#[case::version_zero(0)]
#[case::later_version(23)]
#[tokio::test]
async fn imported_archives_preserve_history_and_findings_at_the_original_version(
    #[future(awt)] database: Database,
    lens: Lens,
    job: Job,
    finding: Finding,
    #[case] version: i64,
) {
    let parent = Lens { version, ..lens };
    let job = Job {
        findings: Some(vec![finding.clone()]),
        ..job
    };
    database
        .seed(&record_key("lens", &[&parent.id]), stored(&parent))
        .await;
    database
        .seed(
            &record_key("run", &[&parent.id, &job.id]),
            archived(&parent, &job),
        )
        .await;
    let repository = database.repository();
    assert_eq!(
        repository.get(&parent.id).await.unwrap().unwrap().version,
        version
    );
    let history = repository.jobs(&parent.id, 0).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id, job.id);
    assert_eq!(
        repository
            .job(&parent.id, &job.id)
            .await
            .unwrap()
            .unwrap()
            .id,
        job.id
    );
    assert_eq!(
        value(
            repository
                .finding_runs(&parent.id, std::slice::from_ref(&finding.id))
                .await
                .unwrap()
        ),
        json!([{"finding_id": finding.id, "job_id": job.id}])
    );
}

#[rstest]
#[case::unpublished(false)]
#[case::future_version(true)]
#[tokio::test]
async fn finding_runs_hide_unpublished_or_future_archives(
    #[future(awt)] database: Database,
    lens: Lens,
    job: Job,
    finding: Finding,
    #[case] publish: bool,
) {
    let repository = database.repository();
    repository.create(&lens).await.unwrap();
    let job = Job {
        findings: Some(vec![finding.clone()]),
        ..job
    };
    let future = Lens {
        version: lens.version + 1,
        ..lens.clone()
    };
    let prepared = database
        .store
        .prepare(vec![Change {
            previous: Snapshot::empty(record_key("run", &[&lens.id, &job.id])),
            value: archived(&future, &job),
        }])
        .await
        .unwrap();
    if publish {
        database.store.publish(&prepared).await.unwrap();
    }
    assert!(
        repository
            .finding_runs(&lens.id, &[finding.id])
            .await
            .unwrap()
            .is_empty()
    );
}

#[rstest]
#[tokio::test]
async fn finding_runs_hide_obsolete_parent_and_unpublished_archive_versions(
    #[future(awt)] database: Database,
    lens: Lens,
    job: Job,
    finding: Finding,
) {
    let old_job = Job {
        findings: Some(vec![finding.clone()]),
        ..job
    };
    let original = Lens {
        jobs: vec![old_job.clone()],
        ..lens
    };
    let repository = database.repository();
    repository.create(&original).await.unwrap();
    let previous = database
        .store
        .read(&record_key("lens", &[&original.id]))
        .await
        .unwrap();
    let removed = Lens {
        jobs: vec![],
        version: original.version + 1,
        ..original.clone()
    };
    let _prepared = database
        .store
        .prepare(vec![
            Change {
                previous,
                value: stored(&removed),
            },
            Change {
                previous: Snapshot::empty(record_key("run", &[&original.id, &old_job.id])),
                value: archived(&removed, &old_job),
            },
        ])
        .await
        .unwrap();
    let updated_job = Job {
        findings: Some(vec![Finding {
            id: "new-finding".into(),
            ..finding.clone()
        }]),
        ..old_job
    };
    repository
        .replace(
            &original,
            &Lens {
                jobs: vec![updated_job.clone()],
                ..original.clone()
            },
        )
        .await
        .unwrap();
    assert!(
        repository
            .finding_runs(&original.id, &[finding.id])
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        value(
            repository
                .finding_runs(&original.id, &["new-finding".into()])
                .await
                .unwrap()
        ),
        json!([{"finding_id":"new-finding","job_id":updated_job.id}])
    );
}

#[rstest]
#[tokio::test]
async fn finding_runs_fail_on_unavailable_storage_and_skip_empty_requests(
    #[future(awt)] database: Database,
    lens: Lens,
) {
    database.sql("DROP TABLE lens_state_heads SYNC").await;
    assert!(
        database
            .repository()
            .finding_runs(&lens.id, &[])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        database
            .repository()
            .finding_runs(&lens.id, &["finding".into()])
            .await,
        Err(RepositoryError::Unavailable(_))
    ));
}
