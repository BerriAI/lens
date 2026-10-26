use super::support::{Database, database, isolated_database, job, lens, value};
use chrono::Utc;
use lens_contract::{
    investigations::Lens,
    worker::{Findings, Job, JobStatus},
};
use lens_investigations::{AnalysisCheckpoint, CheckpointError, LensRepository};
use rstest::{fixture, rstest};

#[fixture]
fn active(lens: Lens, job: Job) -> Lens {
    Lens {
        jobs: vec![Job {
            status: JobStatus::Running,
            worker_id: Some("worker".into()),
            attempts: 1,
            lease_until: Some(Utc::now() + chrono::Duration::minutes(5)),
            ..job
        }],
        ..lens
    }
}

#[rstest]
#[tokio::test]
async fn checkpoints_survive_database_restart_and_do_not_rewrite_job_state(
    #[future(awt)] isolated_database: Database,
    active: Lens,
) {
    let database = isolated_database;
    let repository = database.repository();
    repository.create(&active).await.unwrap();
    let checkpoint = AnalysisCheckpoint::Checking {
        result: Findings::default(),
    };
    let saved = repository
        .save_analysis_checkpoint(&active.id, &active.jobs[0], "fingerprint", &checkpoint)
        .await
        .unwrap();
    assert_eq!(value(saved), value(&checkpoint));
    assert_eq!(
        value(repository.get(&active.id).await.unwrap().unwrap()),
        value(&active)
    );
    let restarted = database.restart().await;
    assert_eq!(
        value(
            restarted
                .analysis_checkpoint(&active.id, "fingerprint")
                .await
                .unwrap()
                .unwrap()
        ),
        value(&checkpoint)
    );
    assert!(
        restarted
            .analysis_checkpoint("another-lens", "fingerprint")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        restarted
            .analysis_checkpoint(&active.id, "changed-evidence")
            .await
            .unwrap()
            .is_none()
    );
}

#[rstest]
#[case::worker(true, false, false, false)]
#[case::attempt(false, true, false, false)]
#[case::expired(false, false, true, false)]
#[case::cancelled(false, false, false, true)]
#[tokio::test]
async fn stale_workers_cannot_save_or_replace_checkpoints(
    #[future(awt)] database: Database,
    active: Lens,
    #[case] worker: bool,
    #[case] attempt: bool,
    #[case] expired: bool,
    #[case] cancelled: bool,
) {
    let repository = database.repository();
    repository.create(&active).await.unwrap();
    let checkpoint = AnalysisCheckpoint::Grouping {
        candidates: vec![],
        batches: 1,
    };
    repository
        .save_analysis_checkpoint(&active.id, &active.jobs[0], "group", &checkpoint)
        .await
        .unwrap();
    let mut changed = active.clone();
    if worker {
        changed.jobs[0].worker_id = Some("replacement".into());
    }
    if attempt {
        changed.jobs[0].attempts += 1;
    }
    if expired {
        changed.jobs[0].lease_until = Some(Utc::now() - chrono::Duration::seconds(1));
    }
    if cancelled {
        changed.jobs[0].status = JobStatus::Cancelled;
    }
    repository.replace(&active, &changed).await.unwrap();
    let result = repository
        .save_analysis_checkpoint(&active.id, &active.jobs[0], "group", &checkpoint)
        .await;
    assert!(matches!(result, Err(CheckpointError::Ownership)));
    let result = repository
        .save_analysis_checkpoint(&active.id, &active.jobs[0], "new", &checkpoint)
        .await;
    assert!(matches!(result, Err(CheckpointError::Ownership)));
    assert!(
        repository
            .analysis_checkpoint(&active.id, "new")
            .await
            .unwrap()
            .is_none()
    );
}

#[rstest]
#[tokio::test]
async fn checkpoint_replay_preserves_the_first_published_candidate_set(
    #[future(awt)] database: Database,
    active: Lens,
) {
    let repository = database.repository();
    repository.create(&active).await.unwrap();
    let first = AnalysisCheckpoint::Grouping {
        candidates: vec![],
        batches: 1,
    };
    let second = AnalysisCheckpoint::Grouping {
        candidates: vec![],
        batches: 2,
    };
    repository
        .save_analysis_checkpoint(&active.id, &active.jobs[0], "group", &first)
        .await
        .unwrap();
    let replay = repository
        .save_analysis_checkpoint(&active.id, &active.jobs[0], "group", &second)
        .await
        .unwrap();
    assert_eq!(value(replay), value(first));
}

#[rstest]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unconfirmed_checkpoint_is_recovered_without_republishing(
    #[future(awt)] database: Database,
    active: Lens,
) {
    use litellm_http::Client;
    use litellm_storage_clickhouse::investigations::Investigations;
    use litellm_storage_clickhouse::{Connection, state::ClickHouseState};
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};
    database.repository().create(&active).await.unwrap();
    let server = MockServer::start().await;
    let destination = database.url();
    let publications = Arc::new(AtomicUsize::new(0));
    let count = publications.clone();
    Mock::given(wiremock::matchers::method("POST"))
        .respond_with(move |request: &Request| {
            let mut target = destination.clone();
            target
                .query_pairs_mut()
                .extend_pairs(request.url.query_pairs());
            let body = request.body.clone();
            let response = std::thread::spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(async {
                        let response = Client::no_redirect_for_test()
                            .post(target)
                            .header("Content-Length", body.len().to_string())
                            .body(body)
                            .send()
                            .await
                            .unwrap();
                        let status = response.status().as_u16();
                        (status, response.text().await.unwrap())
                    })
            })
            .join()
            .unwrap();
            let publication = request.url.query_pairs().any(|(name, query)| {
                name == "query" && query.starts_with("ALTER TABLE lens_state_heads")
            }) && request
                .url
                .query_pairs()
                .any(|(name, keys)| name == "param_keys" && keys.contains("analysis-checkpoint/"));
            let template = ResponseTemplate::new(response.0).set_body_string(response.1);
            if publication && count.fetch_add(1, Ordering::SeqCst) == 0 {
                return template.set_delay(Duration::from_secs(15));
            }
            template
        })
        .mount(&server)
        .await;
    let repository = Investigations(ClickHouseState::new(
        Client::no_redirect_for_test(),
        Connection::parse(&server.uri()).unwrap(),
    ));
    let checkpoint = AnalysisCheckpoint::Checking {
        result: Findings::default(),
    };
    let saved = repository
        .save_analysis_checkpoint(&active.id, &active.jobs[0], "candidate", &checkpoint)
        .await
        .unwrap();
    assert_eq!(value(saved), value(checkpoint));
    assert_eq!(publications.load(Ordering::SeqCst), 1);
    assert_eq!(
        value(
            database
                .repository()
                .get(&active.id)
                .await
                .unwrap()
                .unwrap()
        ),
        value(active)
    );
}
