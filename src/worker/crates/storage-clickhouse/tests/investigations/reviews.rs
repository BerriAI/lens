use chrono::{DateTime, Duration, Utc};
use lens_contract::{
    investigations::{Lens, Public},
    worker::{Job, JobStatus, Progress, Review, ReviewVersion},
};
use lens_investigations::{
    CheckpointError, LensRepository, RepositoryError, criteria_key, current_job,
};
use rstest::{fixture, rstest};
use serde_json::{Value, json};

use super::support::{Database, database, isolated_database, job, lens, now, record_key, value};

#[fixture]
fn recorded() -> Value {
    serde_json::from_str(include_str!(
        "../../../contract/tests/fixtures/investigations_public.json"
    ))
    .unwrap()
}

#[fixture]
fn assigned(job: Job, now: DateTime<Utc>, recorded: Value) -> Job {
    Job {
        status: JobStatus::Running,
        worker_id: Some("worker".into()),
        attempts: 1,
        lease_until: Some(now + Duration::minutes(1)),
        sample: Some(serde_json::from_value(recorded["sample"].clone()).unwrap()),
        ..job
    }
}

#[fixture]
fn active(lens: Lens, assigned: Job) -> Lens {
    Lens {
        jobs: vec![assigned],
        ..lens
    }
}

#[fixture]
fn review(recorded: Value, now: DateTime<Utc>) -> Review {
    Review {
        at: now,
        content_version: "version-1".into(),
        ..serde_json::from_value(recorded["review"].clone()).unwrap()
    }
}

fn review_key(lens: &Lens, job: &Job, review: &Review) -> String {
    record_key(
        "review",
        &[
            &lens.id,
            &criteria_key(&job.settings).unwrap(),
            &review.execution_id,
        ],
    )
}

#[rstest]
#[tokio::test]
async fn progress_publishes_summary_and_full_cached_review_together(
    #[future(awt)] database: Database,
    active: Lens,
    assigned: Job,
    review: Review,
    now: DateTime<Utc>,
) {
    let repository = database.repository();
    repository.create(&active).await.unwrap();
    let body = Progress {
        stage: Some("Reading traces".into()),
        review: Some(review.clone()),
        ..Progress::default()
    };
    let updated = repository
        .progress(&active.id, &assigned, &body, now)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.version, active.version + 1);
    let job = current_job(&updated).unwrap();
    assert_eq!(job.reviewed, 1);
    assert_eq!(job.stage, "Reading traces");
    assert_eq!(job.reviews[0].execution_id, review.execution_id);
    assert!(job.reviews[0].extraction.is_none());
    assert!(job.reviews[0].content_version.is_empty());
    assert!(job.lease_until.unwrap() >= now + Duration::minutes(5));
    assert!(job.lease_until.unwrap() < now + Duration::minutes(6));
    assert_eq!(job.steps.last().unwrap().label.as_str(), "Reading traces");
    let cache = repository.reviews(&active.id, &assigned).await.unwrap();
    assert_eq!(value(Public(cache)), value(Public(vec![review.clone()])));
    let stored = database
        .store
        .read(&review_key(&active, &assigned, &review))
        .await
        .unwrap();
    assert_eq!(stored.value, value(Public(&review)));
    assert!(stored.value.get("tool_calls").is_some());
    assert_eq!(
        value(repository.get(&active.id).await.unwrap().unwrap()),
        value(updated)
    );
}

#[rstest]
#[case::new_review((false, true, true, true))]
#[case::reused((true, true, true, false))]
#[case::without_extraction((false, false, true, false))]
#[case::without_content_version((false, true, false, false))]
#[tokio::test]
async fn only_reusable_full_reviews_enter_cache(
    #[future(awt)] database: Database,
    active: Lens,
    assigned: Job,
    review: Review,
    now: DateTime<Utc>,
    #[case] options: (bool, bool, bool, bool),
) {
    let (reused, extraction, version, cached) = options;
    let review = Review {
        reused,
        extraction: if extraction {
            review.extraction.clone()
        } else {
            None
        },
        content_version: if version {
            review.content_version.clone()
        } else {
            String::new()
        },
        ..review
    };
    let repository = database.repository();
    repository.create(&active).await.unwrap();
    let updated = repository
        .progress(
            &active.id,
            &assigned,
            &Progress {
                review: Some(review),
                ..Progress::default()
            },
            now,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current_job(&updated).unwrap().reviewed, 1);
    assert_eq!(
        repository
            .reviews(&active.id, &assigned)
            .await
            .unwrap()
            .len(),
        usize::from(cached)
    );
}

#[rstest]
#[tokio::test]
async fn checkpoint_insert_failure_cannot_advance_visible_job_progress(
    #[future(awt)] database: Database,
    active: Lens,
    assigned: Job,
    review: Review,
    now: DateTime<Utc>,
) {
    let repository = database.repository();
    repository.create(&active).await.unwrap();
    let lens_key = record_key("lens", &[&active.id]);
    let previous = database.store.read(&lens_key).await.unwrap();
    database.sql("ALTER TABLE lens_state_blobs ADD CONSTRAINT reject_review CHECK NOT startsWith(key, 'review/')").await;
    let result = repository
        .progress(
            &active.id,
            &assigned,
            &Progress {
                review: Some(review.clone()),
                ..Progress::default()
            },
            now,
        )
        .await;
    assert!(matches!(
        result,
        Err(CheckpointError::Store(RepositoryError::Unavailable(_)))
    ));
    assert_eq!(database.store.read(&lens_key).await.unwrap(), previous);
    assert!(
        database
            .store
            .read(&review_key(&active, &assigned, &review))
            .await
            .unwrap()
            .value
            .is_null()
    );
}

enum AssignmentFault {
    JobId,
    Worker,
    Attempt,
    Queued,
    Completed,
    Missing,
    NoLease,
    Expired,
    Boundary,
}

#[rstest]
#[case::job_id(AssignmentFault::JobId)]
#[case::worker(AssignmentFault::Worker)]
#[case::attempt(AssignmentFault::Attempt)]
#[case::queued(AssignmentFault::Queued)]
#[case::completed(AssignmentFault::Completed)]
#[case::missing(AssignmentFault::Missing)]
#[case::no_lease(AssignmentFault::NoLease)]
#[case::expired(AssignmentFault::Expired)]
#[case::lease_boundary(AssignmentFault::Boundary)]
#[tokio::test]
async fn stale_assignment_cannot_publish_progress_or_cached_review(
    #[future(awt)] database: Database,
    active: Lens,
    assigned: Job,
    review: Review,
    now: DateTime<Utc>,
    #[case] fault: AssignmentFault,
) {
    let current = match fault {
        AssignmentFault::JobId => Some(Job {
            id: "reassigned".into(),
            ..assigned.clone()
        }),
        AssignmentFault::Worker => Some(Job {
            worker_id: Some("other-worker".into()),
            ..assigned.clone()
        }),
        AssignmentFault::Attempt => Some(Job {
            attempts: assigned.attempts + 1,
            ..assigned.clone()
        }),
        AssignmentFault::Queued => Some(Job {
            status: JobStatus::Queued,
            ..assigned.clone()
        }),
        AssignmentFault::Completed => Some(Job {
            status: JobStatus::Completed,
            ..assigned.clone()
        }),
        AssignmentFault::Missing => None,
        AssignmentFault::NoLease => Some(Job {
            lease_until: None,
            ..assigned.clone()
        }),
        AssignmentFault::Expired => Some(Job {
            lease_until: Some(now - Duration::microseconds(1)),
            ..assigned.clone()
        }),
        AssignmentFault::Boundary => Some(Job {
            lease_until: Some(now),
            ..assigned.clone()
        }),
    };
    let active = Lens {
        jobs: current.into_iter().collect(),
        ..active
    };
    let repository = database.repository();
    repository.create(&active).await.unwrap();
    let result = repository
        .progress(
            &active.id,
            &assigned,
            &Progress {
                review: Some(review.clone()),
                ..Progress::default()
            },
            now,
        )
        .await;
    assert!(matches!(result, Err(CheckpointError::Ownership)));
    assert_eq!(
        value(repository.get(&active.id).await.unwrap().unwrap()),
        value(&active)
    );
    assert!(
        database
            .store
            .read(&review_key(&active, &assigned, &review))
            .await
            .unwrap()
            .value
            .is_null()
    );
}

#[rstest]
#[tokio::test]
async fn missing_lens_and_invalid_progress_preserve_state(
    #[future(awt)] database: Database,
    active: Lens,
    assigned: Job,
    review: Review,
    now: DateTime<Utc>,
) {
    let repository = database.repository();
    let body = Progress {
        review: Some(review.clone()),
        ..Progress::default()
    };
    assert!(
        repository
            .progress(&active.id, &assigned, &body, now)
            .await
            .unwrap()
            .is_none()
    );
    let assigned = Job {
        reviewed: i64::MAX,
        ..assigned
    };
    let active = Lens {
        jobs: vec![assigned.clone()],
        ..active
    };
    repository.create(&active).await.unwrap();
    assert!(matches!(
        repository.progress(&active.id, &assigned, &body, now).await,
        Err(CheckpointError::Invalid(
            lens_investigations::Error::ReviewCount
        ))
    ));
    assert_eq!(
        value(repository.get(&active.id).await.unwrap().unwrap()),
        value(&active)
    );
    assert!(
        database
            .store
            .read(&review_key(&active, &assigned, &review))
            .await
            .unwrap()
            .value
            .is_null()
    );
}

#[rstest]
#[tokio::test]
async fn cached_review_reads_follow_sample_order_and_criteria_isolation(
    #[future(awt)] database: Database,
    active: Lens,
    assigned: Job,
    review: Review,
) {
    let second = Review {
        execution_id: "second 雪/'\"".into(),
        ..review.clone()
    };
    database
        .seed(
            &review_key(&active, &assigned, &review),
            value(Public(&review)),
        )
        .await;
    database
        .seed(
            &review_key(&active, &assigned, &second),
            value(Public(&second)),
        )
        .await;
    let sample = assigned.sample.as_ref().unwrap();
    let job = Job {
        sample: Some(lens_contract::worker::Sample {
            executions: vec![
                lens_contract::worker::Execution {
                    id: second.execution_id.clone(),
                    ..sample.executions[0].clone()
                },
                lens_contract::worker::Execution {
                    id: "missing".into(),
                    ..sample.executions[0].clone()
                },
                sample.executions[0].clone(),
            ],
            ..sample.clone()
        }),
        ..assigned.clone()
    };
    let repository = database.repository();
    assert_eq!(
        value(Public(repository.reviews(&active.id, &job).await.unwrap())),
        value(Public(vec![second, review]))
    );
    assert!(
        repository
            .reviews("other-lens", &job)
            .await
            .unwrap()
            .is_empty()
    );
    let changed = Job {
        settings: lens_contract::worker::LensSettings {
            context: "Different criteria".into(),
            ..job.settings.clone()
        },
        ..job.clone()
    };
    assert!(
        repository
            .reviews(&active.id, &changed)
            .await
            .unwrap()
            .is_empty()
    );
    let no_sample = Job {
        sample: None,
        ..job
    };
    assert!(
        repository
            .reviews(&active.id, &no_sample)
            .await
            .unwrap()
            .is_empty()
    );
}

#[rstest]
#[case::same_version("version-1", true)]
#[case::stale_version("older", false)]
#[tokio::test]
async fn completion_marks_only_matching_content_version(
    #[future(awt)] database: Database,
    active: Lens,
    assigned: Job,
    review: Review,
    #[case] content_version: &str,
    #[case] consolidated: bool,
) {
    let key = review_key(&active, &assigned, &review);
    database.seed(&key, value(Public(&review))).await;
    let repository = database.repository();
    let versions = vec![
        ReviewVersion {
            execution_id: review.execution_id.clone(),
            content_version: content_version.into(),
        },
        ReviewVersion {
            execution_id: "missing".into(),
            content_version: "absent".into(),
        },
    ];
    repository
        .complete_reviews(&active.id, &assigned, &versions)
        .await
        .unwrap();
    let cached = repository.reviews(&active.id, &assigned).await.unwrap();
    assert_eq!(cached[0].consolidated, consolidated);
    assert_eq!(cached[0].content_version, review.content_version);
    let previous = database.store.read(&key).await.unwrap();
    repository
        .complete_reviews(&active.id, &assigned, &versions)
        .await
        .unwrap();
    repository
        .complete_reviews(&active.id, &assigned, &[])
        .await
        .unwrap();
    assert_eq!(database.store.read(&key).await.unwrap(), previous);
}

#[rstest]
#[tokio::test]
async fn repeated_completion_identity_uses_last_expected_version(
    #[future(awt)] database: Database,
    active: Lens,
    assigned: Job,
    review: Review,
) {
    database
        .seed(
            &review_key(&active, &assigned, &review),
            value(Public(&review)),
        )
        .await;
    let versions = [
        ReviewVersion {
            execution_id: review.execution_id.clone(),
            content_version: review.content_version,
        },
        ReviewVersion {
            execution_id: review.execution_id,
            content_version: "newer".into(),
        },
    ];
    database
        .repository()
        .complete_reviews(&active.id, &assigned, &versions)
        .await
        .unwrap();
    assert!(
        !database
            .repository()
            .reviews(&active.id, &assigned)
            .await
            .unwrap()[0]
            .consolidated
    );
}

#[rstest]
#[tokio::test]
async fn completion_batch_failure_cannot_partially_consolidate_reviews(
    #[future(awt)] database: Database,
    active: Lens,
    assigned: Job,
    review: Review,
) {
    let second = Review {
        execution_id: "second".into(),
        ..review.clone()
    };
    let first_key = review_key(&active, &assigned, &review);
    let second_key = review_key(&active, &assigned, &second);
    database.seed(&first_key, value(Public(&review))).await;
    database.seed(&second_key, value(Public(&second))).await;
    let keys = [&*first_key, &*second_key];
    let previous = database.store.read_many(&keys).await.unwrap();
    database.sql("ALTER TABLE lens_state_blobs ADD CONSTRAINT reject_second_completion CHECK NOT (startsWith(key, 'review/') AND JSONExtractBool(data, 'consolidated') AND JSONExtractString(data, 'execution_id')='second')").await;
    let versions = [
        ReviewVersion {
            execution_id: review.execution_id,
            content_version: review.content_version,
        },
        ReviewVersion {
            execution_id: second.execution_id,
            content_version: second.content_version,
        },
    ];
    assert!(matches!(
        database
            .repository()
            .complete_reviews(&active.id, &assigned, &versions)
            .await,
        Err(RepositoryError::Unavailable(_))
    ));
    assert_eq!(database.store.read_many(&keys).await.unwrap(), previous);
}

async fn concurrent_progress(
    database: &Database,
    lens_id: &str,
    assigned: &Job,
    review: &Review,
    now: DateTime<Utc>,
) -> Vec<Result<Option<Lens>, CheckpointError>> {
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(16));
    let mut tasks = tokio::task::JoinSet::new();
    for index in 0..16 {
        let repository = database.repository();
        let barrier = barrier.clone();
        let lens_id = lens_id.to_owned();
        let assigned = assigned.clone();
        let review = Review {
            name: format!("Review {index}"),
            content_version: format!("version-{index}"),
            ..review.clone()
        };
        tasks.spawn(async move {
            barrier.wait().await;
            repository
                .progress(
                    &lens_id,
                    &assigned,
                    &Progress {
                        review: Some(review),
                        ..Progress::default()
                    },
                    now,
                )
                .await
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
async fn concurrent_progress_retries_without_losing_or_duplicating_reviews(
    #[future(awt)] database: Database,
    active: Lens,
    assigned: Job,
    review: Review,
    now: DateTime<Utc>,
) {
    let repository = database.repository();
    repository.create(&active).await.unwrap();
    let results = concurrent_progress(&database, &active.id, &assigned, &review, now).await;
    assert_eq!(
        results
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .into_iter()
            .flatten()
            .count(),
        16
    );
    let lens = repository.get(&active.id).await.unwrap().unwrap();
    let job = current_job(&lens).unwrap();
    assert_eq!(job.reviewed, 16);
    assert_eq!(job.reviews.len(), 16);
    assert_eq!(
        job.reviews
            .iter()
            .map(|review| &review.name)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        16
    );
    let cached = repository.reviews(&active.id, &assigned).await.unwrap();
    assert_eq!(cached.len(), 1);
    assert_eq!(cached[0].name, job.reviews.last().unwrap().name);
    assert_eq!(lens.version, active.version + 16);
}

#[rstest]
#[tokio::test]
async fn review_checkpoint_survives_abrupt_restart(
    #[future(awt)] isolated_database: Database,
    active: Lens,
    assigned: Job,
    review: Review,
    now: DateTime<Utc>,
) {
    let repository = isolated_database.repository();
    repository.create(&active).await.unwrap();
    let published = repository
        .progress(
            &active.id,
            &assigned,
            &Progress {
                review: Some(review.clone()),
                ..Progress::default()
            },
            now,
        )
        .await
        .unwrap()
        .unwrap();
    let reopened = isolated_database.restart().await;
    assert_eq!(
        value(reopened.get(&active.id).await.unwrap().unwrap()),
        value(published)
    );
    assert_eq!(
        value(Public(
            reopened.reviews(&active.id, &assigned).await.unwrap()
        )),
        value(Public(vec![review]))
    );
}

#[rstest]
#[tokio::test]
async fn malformed_review_and_unavailable_storage_fail_closed(
    #[future(awt)] database: Database,
    active: Lens,
    assigned: Job,
    review: Review,
    now: DateTime<Utc>,
) {
    database
        .seed(
            &review_key(&active, &assigned, &review),
            json!({"execution_id": review.execution_id}),
        )
        .await;
    let repository = database.repository();
    let versions = [ReviewVersion {
        execution_id: review.execution_id,
        content_version: review.content_version,
    }];
    assert!(matches!(
        repository.reviews(&active.id, &assigned).await,
        Err(RepositoryError::Unavailable(_))
    ));
    assert!(matches!(
        repository
            .complete_reviews(&active.id, &assigned, &versions)
            .await,
        Err(RepositoryError::Unavailable(_))
    ));
    database.sql("DROP TABLE lens_state_heads SYNC").await;
    assert!(matches!(
        repository.reviews(&active.id, &assigned).await,
        Err(RepositoryError::Unavailable(_))
    ));
    assert!(matches!(
        repository
            .complete_reviews(&active.id, &assigned, &versions)
            .await,
        Err(RepositoryError::Unavailable(_))
    ));
    assert!(matches!(
        repository
            .progress(&active.id, &assigned, &Progress::default(), now)
            .await,
        Err(CheckpointError::Store(RepositoryError::Unavailable(_)))
    ));
}
