use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use chrono::Utc;
use lens_contract::{
    investigations::{BudgetReservation, Lens},
    worker::{Findings, Job, JobStatus, Progress},
};
use lens_inference::{BUDGET_LEASE, renew_reservation, reserve_attempt, settle_amount};
use lens_investigations::{AnalysisCheckpoint, CheckpointError, LensRepository, RepositoryError};
use litellm_storage_clickhouse::investigations::Investigations;
use rstest::{fixture, rstest};
use tokio::{sync::Notify, task::JoinHandle};

use super::support::{Database, LensRead, database, job, lens, record_key, slow_lens_reads, value};

#[fixture]
fn active(lens: Lens, job: Job) -> Lens {
    let now = Utc::now();
    Lens {
        budget_month: now.format("%Y-%m").to_string(),
        jobs: vec![Job {
            status: JobStatus::Running,
            worker_id: Some("worker".into()),
            attempts: 1,
            lease_until: Some(now + chrono::Duration::minutes(5)),
            cost: 0.0,
            ..job
        }],
        ..lens
    }
}

fn progress_writer(
    repository: Investigations,
    assigned: Job,
    stop: Arc<AtomicBool>,
    writes: Arc<AtomicUsize>,
    ready: Arc<Notify>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while !stop.load(Ordering::SeqCst) {
            let sequence = writes.fetch_add(1, Ordering::SeqCst);
            let progress = Progress {
                stage: Some(format!("Concurrent trace progress {sequence}")),
                ..Default::default()
            };
            repository
                .progress("lens", &assigned, &progress, Utc::now())
                .await
                .unwrap()
                .unwrap();
            ready.notify_one();
        }
    })
}

#[rstest]
#[case::reserve(false, false)]
#[case::renew(true, false)]
#[case::settle(false, true)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn budget_writes_finish_while_independent_clients_publish_progress(
    #[future(awt)] database: Database,
    active: Lens,
    #[case] renewing: bool,
    #[case] settling: bool,
) {
    let hold = BudgetReservation {
        id: "request".into(),
        job_id: active.jobs[0].id.clone(),
        amount: 1.0,
        month: active.budget_month.clone(),
        expires_at: Some(Utc::now() + BUDGET_LEASE),
    };
    let initial = Lens {
        reservations: if renewing || settling {
            vec![hold.clone()]
        } else {
            vec![]
        },
        ..active
    };
    let repository = database.repository();
    repository.create(&initial).await.unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let writes = Arc::new(AtomicUsize::new(0));
    let ready = Arc::new(Notify::new());
    let assigned = initial.jobs[0].clone();
    let writers: Vec<_> = (0..4)
        .map(|_| {
            progress_writer(
                database.repository(),
                assigned.clone(),
                stop.clone(),
                writes.clone(),
                ready.clone(),
            )
        })
        .collect();
    ready.notified().await;
    let reservation = hold.clone();
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        repository.update_locked("lens", move |current| {
            std::thread::sleep(Duration::from_millis(150));
            if settling {
                Ok(settle_amount(current, &reservation.id, 0.25, None))
            } else if renewing {
                renew_reservation(current, &reservation.id, Utc::now())
            } else {
                reserve_attempt(current, &assigned, "worker", &reservation, Utc::now())
            }
            .map_err(|error| RepositoryError::Unavailable(Box::new(error)))
        }),
    )
    .await;
    stop.store(true, Ordering::SeqCst);
    for writer in writers {
        writer.await.unwrap();
    }
    let updated = result
        .expect("Progress writers starved the budget operation")
        .unwrap()
        .unwrap();
    let saved = database.repository().get("lens").await.unwrap().unwrap();
    assert_eq!(saved.reservations.len(), usize::from(!settling));
    if !settling {
        assert_eq!(saved.reservations[0].id, hold.id);
    }
    assert_eq!(value(&saved.reservations), value(&updated.reservations));
    let charged = if settling { 0.25 } else { 0.0 };
    assert_eq!(saved.spent, initial.spent + charged);
    assert_eq!(saved.jobs[0].cost, initial.jobs[0].cost + charged);
    assert!(
        saved.jobs[0]
            .stage
            .starts_with("Concurrent trace progress ")
    );
    assert!(writes.load(Ordering::SeqCst) > 1);
    if renewing {
        assert!(saved.reservations[0].expires_at > hold.expires_at);
    }
}

fn lease_key() -> String {
    super::support::record_key("lens-writer", &["lens"])
}

async fn wait_released(database: &Database) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if database
                .store
                .read(&lease_key())
                .await
                .unwrap()
                .value
                .is_null()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Writer cleanup did not release its lease");
}

#[rstest]
#[case::noop(false, false)]
#[case::missing(true, false)]
#[case::transform_error(false, true)]
#[tokio::test]
async fn unchanged_or_rejected_updates_release_the_writer_without_changing_data(
    #[future(awt)] database: Database,
    lens: Lens,
    #[case] missing: bool,
    #[case] rejected: bool,
) {
    let repository = database.repository();
    if !missing {
        repository.create(&lens).await.unwrap();
    }
    let key = super::support::record_key("lens", &[&lens.id]);
    let before = database.store.read(&key).await.unwrap();
    let transforms = Arc::new(AtomicUsize::new(0));
    let counted = transforms.clone();
    let result = repository
        .update_locked(&lens.id, move |current| {
            counted.fetch_add(1, Ordering::SeqCst);
            if rejected {
                Err(RepositoryError::Conflict)
            } else {
                Ok(current.clone())
            }
        })
        .await;
    if rejected {
        assert!(matches!(result, Err(RepositoryError::Conflict)));
        assert_eq!(transforms.load(Ordering::SeqCst), 1);
    } else {
        assert_eq!(result.unwrap().is_none(), missing);
    }
    wait_released(&database).await;
    assert_eq!(database.store.read(&key).await.unwrap(), before);
    let following = repository
        .update_locked(&lens.id, |current| {
            Ok::<_, RepositoryError>(Lens {
                spent: 2.0,
                ..current.clone()
            })
        })
        .await
        .unwrap();
    assert_eq!(following.map(|lens| lens.spent), (!missing).then_some(2.0));
}

#[rstest]
#[case::expired(-1, false)]
#[case::live(60, true)]
#[tokio::test]
async fn abandoned_writers_expire_but_live_writers_time_out_without_publishing(
    #[future(awt)] database: Database,
    lens: Lens,
    #[case] seconds: i64,
    #[case] live: bool,
) {
    let repository = database.repository();
    repository.create(&lens).await.unwrap();
    let previous = database.store.read(&lease_key()).await.unwrap();
    database.store.commit(vec![litellm_storage_clickhouse::state::Change {
        previous,
        value: serde_json::json!({"token": "abandoned", "expires_at": Utc::now() + chrono::Duration::seconds(seconds)}),
    }]).await.unwrap();
    let held = database.store.read(&lease_key()).await.unwrap();
    let started = std::time::Instant::now();
    let result = repository
        .update_locked(&lens.id, |current| {
            Ok::<_, RepositoryError>(Lens {
                spent: 2.0,
                ..current.clone()
            })
        })
        .await;
    if live {
        assert!(
            matches!(result, Err(RepositoryError::Conflict)),
            "{result:?}"
        );
        assert!(started.elapsed() >= Duration::from_secs(29));
        assert!(started.elapsed() < Duration::from_secs(34));
        assert_eq!(repository.0.read(&lease_key()).await.unwrap(), held);
        assert_eq!(
            value(repository.get(&lens.id).await.unwrap().unwrap()),
            value(&lens)
        );
    } else {
        assert_eq!(result.unwrap().unwrap().spent, 2.0);
        assert!(
            database
                .store
                .read(&lease_key())
                .await
                .unwrap()
                .value
                .is_null()
        );
    }
}

#[rstest]
#[tokio::test]
async fn a_write_claimed_late_in_its_window_still_fails_within_it_while_the_lease_is_held(
    #[future(awt)] database: Database,
    lens: Lens,
) {
    let repository = database.repository();
    repository.create(&lens).await.unwrap();
    let previous = database.store.read(&lease_key()).await.unwrap();
    database
        .store
        .commit(vec![litellm_storage_clickhouse::state::Change {
            previous,
            value: serde_json::json!({
                "token": "busy-writer",
                "expires_at": Utc::now() + chrono::Duration::seconds(90),
            }),
        }])
        .await
        .unwrap();
    let spend = |amount: f64| {
        let repository = repository.clone();
        tokio::spawn(async move {
            let started = std::time::Instant::now();
            let result = repository
                .update_locked("lens", move |current| {
                    Ok::<_, RepositoryError>(Lens {
                        spent: amount,
                        ..current.clone()
                    })
                })
                .await;
            (result, started.elapsed())
        })
    };
    let first = spend(1.0);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let second = spend(2.0);
    for (result, elapsed) in joined(vec![first, second]).await {
        assert!(
            matches!(result, Err(RepositoryError::Conflict)),
            "{result:?}"
        );
        assert!(elapsed >= Duration::from_secs(29), "{elapsed:?}");
        assert!(elapsed < Duration::from_secs(34), "{elapsed:?}");
    }
    assert_eq!(
        value(repository.get(&lens.id).await.unwrap().unwrap()),
        value(&lens)
    );
}

#[rstest]
#[case::budget(false, false)]
#[case::progress(true, false)]
#[case::reassigned_progress(true, true)]
#[tokio::test]
async fn writer_acquisition_retries_recover_without_bypassing_job_ownership(
    #[future(awt)] database: Database,
    active: Lens,
    #[case] progress: bool,
    #[case] reassigned: bool,
) {
    let assigned = active.jobs[0].clone();
    let initial = if reassigned {
        Lens {
            jobs: vec![Job {
                worker_id: Some("replacement-worker".into()),
                ..assigned.clone()
            }],
            ..active
        }
    } else {
        active
    };
    let repository = database.repository();
    repository.create(&initial).await.unwrap();
    let previous = database.store.read(&lease_key()).await.unwrap();
    database
        .store
        .commit(vec![litellm_storage_clickhouse::state::Change {
            previous,
            value: serde_json::json!({
                "token": "busy-writer",
                "expires_at": Utc::now() + chrono::Duration::seconds(16),
            }),
        }])
        .await
        .unwrap();
    let started = std::time::Instant::now();
    if progress {
        let result = repository
            .progress(
                &initial.id,
                &assigned,
                &Progress {
                    stage: Some("Reviewing after contention".into()),
                    ..Default::default()
                },
                Utc::now(),
            )
            .await;
        if reassigned {
            assert!(matches!(result, Err(CheckpointError::Ownership)));
        } else {
            assert_eq!(
                result.unwrap().unwrap().jobs[0].stage,
                "Reviewing after contention"
            );
        }
    } else {
        let result = repository
            .update_locked(&initial.id, |current| {
                Ok::<_, RepositoryError>(Lens {
                    spent: current.spent + 1.0,
                    ..current.clone()
                })
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.spent, initial.spent + 1.0);
    }
    assert!(started.elapsed() >= Duration::from_secs(15));
    assert!(started.elapsed() < Duration::from_secs(30));
    let saved = repository.get(&initial.id).await.unwrap().unwrap();
    assert_eq!(saved.version, initial.version + i64::from(!reassigned));
    assert_eq!(saved.jobs[0].worker_id, initial.jobs[0].worker_id);
}

struct PausedWrite {
    started: Arc<Notify>,
    calls: Arc<AtomicUsize>,
    release: std::sync::mpsc::Sender<()>,
    task: JoinHandle<Result<Option<Lens>, RepositoryError>>,
}

fn paused_write(repository: Investigations, spent: f64) -> PausedWrite {
    let started = Arc::new(Notify::new());
    let signal = started.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let invocations = calls.clone();
    let (release, wait) = std::sync::mpsc::channel();
    let task = tokio::spawn(async move {
        repository
            .update_locked("lens", move |lens| {
                if invocations.fetch_add(1, Ordering::SeqCst) == 0 {
                    signal.notify_one();
                    wait.recv_timeout(Duration::from_secs(20)).unwrap();
                }
                Ok(Lens {
                    spent: lens.spent + spent,
                    ..lens.clone()
                })
            })
            .await
    });
    PausedWrite {
        started,
        calls,
        release,
        task,
    }
}

#[rstest]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn replaced_writer_retries_from_fresh_state_without_releasing_the_successor_lease(
    #[future(awt)] database: Database,
    lens: Lens,
) {
    database.repository().create(&lens).await.unwrap();
    let first = paused_write(database.repository(), 3.0);
    first.started.notified().await;
    let held = database.store.read(&lease_key()).await.unwrap();
    let mut expired = held.value.clone();
    expired["expires_at"] = value(Utc::now() - chrono::Duration::seconds(1));
    database
        .store
        .commit(vec![litellm_storage_clickhouse::state::Change {
            previous: held,
            value: expired,
        }])
        .await
        .unwrap();
    let successor = paused_write(database.repository(), 5.0);
    successor.started.notified().await;
    let successor_lease = database.store.read(&lease_key()).await.unwrap();
    first.release.send(()).unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!first.task.is_finished());
    assert_eq!(
        database.store.read(&lease_key()).await.unwrap(),
        successor_lease
    );
    assert_eq!(
        value(database.repository().get(&lens.id).await.unwrap().unwrap()),
        value(&lens)
    );
    successor.release.send(()).unwrap();
    assert_eq!(successor.task.await.unwrap().unwrap().unwrap().spent, 5.0);
    assert_eq!(first.task.await.unwrap().unwrap().unwrap().spent, 8.0);
    assert_eq!(first.calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        database
            .repository()
            .get(&lens.id)
            .await
            .unwrap()
            .unwrap()
            .spent,
        8.0
    );
    assert!(
        database
            .store
            .read(&lease_key())
            .await
            .unwrap()
            .value
            .is_null()
    );
}

type Arrivals = Arc<std::sync::Mutex<Vec<usize>>>;

async fn queued_writes(
    repository: &Investigations,
    arrivals: &Arrivals,
    count: usize,
) -> Vec<JoinHandle<Result<Option<Lens>, RepositoryError>>> {
    let mut writes = Vec::new();
    for position in 0..count {
        let repository = repository.clone();
        let arrivals = arrivals.clone();
        writes.push(tokio::spawn(async move {
            repository
                .update_locked("lens", move |lens| {
                    arrivals.lock().unwrap().push(position);
                    Ok(Lens {
                        spent: lens.spent + 1.0,
                        ..lens.clone()
                    })
                })
                .await
        }));
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    writes
}

#[rstest]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn writers_sharing_a_store_take_a_held_lens_in_arrival_order(
    #[future(awt)] database: Database,
    lens: Lens,
) {
    database.repository().create(&lens).await.unwrap();
    let holder = paused_write(database.repository(), 1.0);
    holder.started.notified().await;
    let arrivals = Arrivals::default();
    let writes = queued_writes(&database.repository(), &arrivals, 6).await;
    holder.release.send(()).unwrap();
    holder.task.await.unwrap().unwrap();
    for write in writes {
        write.await.unwrap().unwrap();
    }
    assert_eq!(*arrivals.lock().unwrap(), [0, 1, 2, 3, 4, 5]);
    assert_eq!(
        database
            .repository()
            .get(&lens.id)
            .await
            .unwrap()
            .unwrap()
            .spent,
        lens.spent + 7.0
    );
}

#[rstest]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn abort_during_acquisition_publication_releases_the_committed_owner(
    #[future(awt)] database: Database,
    lens: Lens,
) {
    use litellm_http::Client;
    use litellm_storage_clickhouse::{Connection, state::ClickHouseState};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};
    database.repository().create(&lens).await.unwrap();
    let server = MockServer::start().await;
    let destination = database.url();
    let published = Arc::new(Notify::new());
    let signal = published.clone();
    let delayed = AtomicBool::new(false);
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
            let acquisition = request.url.query_pairs().any(|(name, query)| {
                name == "query" && query.starts_with("ALTER TABLE lens_state_heads")
            });
            let template = ResponseTemplate::new(response.0).set_body_string(response.1);
            if acquisition && !delayed.swap(true, Ordering::SeqCst) {
                signal.notify_one();
                return template.set_delay(Duration::from_secs(30));
            }
            template
        })
        .mount(&server)
        .await;
    let repository = Investigations(ClickHouseState::new(
        Client::no_redirect_for_test(),
        Connection::parse(&server.uri()).unwrap(),
    ));
    let mut task = tokio::spawn(async move {
        repository
            .update_locked("lens", |lens| {
                Ok::<_, RepositoryError>(Lens {
                    spent: 99.0,
                    ..lens.clone()
                })
            })
            .await
    });
    tokio::select! {
        _ = published.notified() => (),
        result = &mut task => panic!("Acquisition ended before publication: {result:?}"),
        _ = tokio::time::sleep(Duration::from_secs(3)) => panic!("Acquisition was not published"),
    }
    assert!(
        !database
            .store
            .read(&lease_key())
            .await
            .unwrap()
            .value
            .is_null()
    );
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    let saved = database
        .repository()
        .update_locked("lens", |lens| {
            Ok::<_, RepositoryError>(Lens {
                spent: 1.0,
                ..lens.clone()
            })
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.spent, 1.0);
    assert_eq!(saved.version, lens.version + 1);
    wait_released(&database).await;
    assert_eq!(
        value(database.repository().get("lens").await.unwrap().unwrap()),
        value(&saved)
    );
}

#[rstest]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unconfirmed_publication_is_not_replayed(#[future(awt)] database: Database, lens: Lens) {
    use litellm_http::Client;
    use litellm_storage_clickhouse::{Connection, state::ClickHouseState};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};
    database.repository().create(&lens).await.unwrap();
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
                .any(|(name, keys)| name == "param_keys" && keys.contains("lens/"));
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
    let result = repository
        .update_locked(&lens.id, |current| {
            Ok::<_, RepositoryError>(Lens {
                spent: current.spent + 1.0,
                ..current.clone()
            })
        })
        .await;
    assert!(matches!(result, Err(RepositoryError::WriteUnconfirmed)));
    assert_eq!(publications.load(Ordering::SeqCst), 1);
    let saved = database.repository().get(&lens.id).await.unwrap().unwrap();
    assert_eq!(saved.spent, lens.spent + 1.0);
    assert_eq!(saved.version, lens.version + 1);
    wait_released(&database).await;
}

#[rstest]
#[case::expired_without_replacement(false)]
#[case::expired_with_replacement(true)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn delayed_publication_is_fenced_by_takeover_and_can_finish_before_a_fence(
    #[future(awt)] database: Database,
    active: Lens,
    #[case] takeover: bool,
) {
    use lens_contract::worker::Review;
    use litellm_http::Client;
    use litellm_storage_clickhouse::{Connection, state::ClickHouseState};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};
    let recorded: serde_json::Value = serde_json::from_str(include_str!(
        "../../../contract/tests/fixtures/investigations_public.json"
    ))
    .unwrap();
    let review = Review {
        at: Utc::now(),
        content_version: "delayed-review".into(),
        ..serde_json::from_value(recorded["review"].clone()).unwrap()
    };
    let review_key = super::support::record_key(
        "review",
        &[
            &active.id,
            &lens_investigations::criteria_key(&active.jobs[0].settings).unwrap(),
            &review.execution_id,
        ],
    );
    database.repository().create(&active).await.unwrap();
    let server = MockServer::start().await;
    let destination = database.url();
    let publishing = Arc::new(Notify::new());
    let signal = publishing.clone();
    let finished = Arc::new(Notify::new());
    let finish = finished.clone();
    let (release, wait) = std::sync::mpsc::channel();
    let wait = std::sync::Mutex::new(wait);
    Mock::given(wiremock::matchers::method("POST"))
        .respond_with(move |request: &Request| {
            let publication = request.url.query_pairs().any(|(name, query)| {
                name == "query" && query.starts_with("ALTER TABLE lens_state_heads")
            }) && request
                .url
                .query_pairs()
                .any(|(name, keys)| name == "param_keys" && keys.contains("review/"));
            if publication {
                signal.notify_one();
                wait.lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(20))
                    .unwrap();
            }
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
            if publication {
                finish.notify_one();
            }
            ResponseTemplate::new(response.0).set_body_string(response.1)
        })
        .mount(&server)
        .await;
    let repository = Investigations(ClickHouseState::new(
        Client::no_redirect_for_test(),
        Connection::parse(&server.uri()).unwrap(),
    ));
    let assigned = active.jobs[0].clone();
    let mut task = tokio::spawn(async move {
        repository
            .progress(
                "lens",
                &assigned,
                &Progress {
                    stage: Some("Delayed checkpoint".into()),
                    review: Some(review),
                    ..Default::default()
                },
                Utc::now(),
            )
            .await
    });
    tokio::select! {
        _ = publishing.notified() => (),
        result = &mut task => panic!("Progress ended before publication: {result:?}"),
        _ = tokio::time::sleep(Duration::from_secs(3)) => panic!("Progress was not published"),
    }
    let owner = database.store.read(&lease_key()).await.unwrap();
    let expiry: chrono::DateTime<Utc> =
        serde_json::from_value(owner.value["expires_at"].clone()).unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    tokio::time::sleep(
        (expiry - Utc::now()).to_std().unwrap_or_default() + Duration::from_millis(50),
    )
    .await;
    if takeover {
        let next = database
            .repository()
            .update_locked("lens", |lens| {
                Ok::<_, RepositoryError>(Lens {
                    spent: 4.0,
                    ..lens.clone()
                })
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(next.spent, 4.0);
    }
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), finished.notified())
        .await
        .unwrap();
    let saved = database.repository().get("lens").await.unwrap().unwrap();
    let checkpoint = database.store.read(&review_key).await.unwrap();
    assert_eq!(saved.version, active.version + 1);
    if takeover {
        assert_eq!(saved.spent, 4.0);
        assert_eq!(saved.jobs[0].stage, active.jobs[0].stage);
        assert!(checkpoint.value.is_null());
    } else {
        assert_eq!(saved.jobs[0].stage, "Delayed checkpoint");
        assert_eq!(checkpoint.value["content_version"], "delayed-review");
    }
    assert!(
        database
            .store
            .read(&lease_key())
            .await
            .unwrap()
            .value
            .is_null()
    );
}

#[rstest]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn expired_owner_reacquires_before_recomputing_and_publishing(
    #[future(awt)] database: Database,
    lens: Lens,
) {
    database.repository().create(&lens).await.unwrap();
    let writer = paused_write(database.repository(), 3.0);
    writer.started.notified().await;
    let owner = database.store.read(&lease_key()).await.unwrap();
    let expiry: chrono::DateTime<Utc> =
        serde_json::from_value(owner.value["expires_at"].clone()).unwrap();
    tokio::time::sleep(
        (expiry - Utc::now()).to_std().unwrap_or_default() + Duration::from_millis(50),
    )
    .await;
    writer.release.send(()).unwrap();
    let result = writer.task.await.unwrap().unwrap().unwrap();
    assert_eq!(result.spent, lens.spent + 3.0);
    assert_eq!(result.version, lens.version + 1);
    assert_eq!(writer.calls.load(Ordering::SeqCst), 2);
    wait_released(&database).await;
    assert_eq!(
        value(database.repository().get(&lens.id).await.unwrap().unwrap()),
        value(&result)
    );
}

#[rstest]
#[tokio::test]
async fn locked_update_cannot_rename_a_lens(#[future(awt)] database: Database, lens: Lens) {
    database.repository().create(&lens).await.unwrap();
    let result = database
        .repository()
        .update_locked(&lens.id, |lens| {
            Ok::<_, RepositoryError>(Lens {
                id: "other".into(),
                ..lens.clone()
            })
        })
        .await;
    assert!(matches!(result, Err(RepositoryError::Unavailable(_))));
    wait_released(&database).await;
    assert_eq!(
        value(database.repository().get(&lens.id).await.unwrap().unwrap()),
        value(&lens)
    );
    assert!(database.repository().get("other").await.unwrap().is_none());
}

#[rstest]
#[case::progress(false, false)]
#[case::review(true, false)]
#[case::persistent_delay(false, true)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unconfirmed_progress_replays_only_without_a_review(
    #[future(awt)] database: Database,
    active: Lens,
    #[case] with_review: bool,
    #[case] always_slow: bool,
) {
    let lens = active;
    use litellm_http::Client;
    use litellm_storage_clickhouse::{Connection, state::ClickHouseState};
    use wiremock::{Mock, MockServer, Request, ResponseTemplate};
    database.repository().create(&lens).await.unwrap();
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
                .any(|(name, keys)| name == "param_keys" && keys.contains("lens/"));
            let template = ResponseTemplate::new(response.0).set_body_string(response.1);
            if publication && (count.fetch_add(1, Ordering::SeqCst) == 0 || always_slow) {
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
    let recorded: serde_json::Value = serde_json::from_str(include_str!(
        "../../../contract/tests/fixtures/investigations_public.json"
    ))
    .unwrap();
    let progress = Progress {
        stage: Some("Checking original evidence".into()),
        coverage: Some(lens_contract::worker::Coverage {
            investigated: 2,
            ..Default::default()
        }),
        review: with_review.then(|| serde_json::from_value(recorded["review"].clone()).unwrap()),
        ..Default::default()
    };
    let result = repository
        .progress(&lens.id, &lens.jobs[0], &progress, Utc::now())
        .await;
    if with_review || always_slow {
        assert!(matches!(
            result,
            Err(CheckpointError::Store(RepositoryError::WriteUnconfirmed))
        ));
        if with_review {
            assert_eq!(publications.load(Ordering::SeqCst), 1);
        } else {
            assert!(publications.load(Ordering::SeqCst) >= 2);
        }
    } else {
        assert_eq!(result.unwrap().unwrap().jobs[0].coverage.investigated, 2);
        assert_eq!(publications.load(Ordering::SeqCst), 2);
    }
    let saved = database.repository().get(&lens.id).await.unwrap().unwrap();
    assert_eq!(saved.jobs[0].stage, "Checking original evidence");
    assert_eq!(saved.jobs[0].coverage.investigated, 2);
    assert_eq!(
        saved.jobs[0].reviewed,
        lens.jobs[0].reviewed + i64::from(with_review)
    );
    assert_eq!(saved.jobs[0].steps.len(), lens.jobs[0].steps.len() + 1);
    wait_released(&database).await;
}

type UnpolledWrite<'a> = Pin<Box<dyn Future<Output = Result<(), CheckpointError>> + Send + 'a>>;

fn unpolled_write<'a>(
    repository: &'a Investigations,
    assigned: &'a Job,
    checkpointing: bool,
) -> UnpolledWrite<'a> {
    if checkpointing {
        return Box::pin(async move {
            let checkpoint = AnalysisCheckpoint::Checking {
                result: Findings::default(),
            };
            repository
                .save_analysis_checkpoint("lens", assigned, "unpolled", &checkpoint)
                .await
                .map(drop)
        });
    }
    Box::pin(async move {
        let progress = Progress {
            stage: Some("Unpolled progress".into()),
            ..Default::default()
        };
        repository
            .progress("lens", assigned, &progress, Utc::now())
            .await
            .map(drop)
    })
}

#[rstest]
#[case::progress(false)]
#[case::checkpoint(true)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn progress_saves_while_an_earlier_writer_of_the_lens_is_not_polled(
    #[future(awt)] database: Database,
    active: Lens,
    #[case] checkpointing: bool,
) {
    let repository = database.repository();
    repository.create(&active).await.unwrap();
    let assigned = &active.jobs[0];
    let mut unpolled = unpolled_write(&repository, assigned, checkpointing);
    assert!(
        tokio::time::timeout(Duration::ZERO, &mut unpolled)
            .await
            .is_err()
    );
    let progress = Progress {
        stage: Some("Polled progress".into()),
        ..Default::default()
    };
    let saved = repository
        .progress("lens", assigned, &progress, Utc::now())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.jobs[0].stage, "Polled progress");
    unpolled.await.unwrap();
}

fn grouping(index: u8) -> AnalysisCheckpoint {
    AnalysisCheckpoint::Grouping {
        candidates: vec![],
        batches: i64::from(index),
    }
}

async fn joined<T>(tasks: Vec<JoinHandle<T>>) -> Vec<T> {
    let mut results = Vec::new();
    for task in tasks {
        results.push(task.await.unwrap());
    }
    results
}

#[rstest]
#[case::analysis_checkpoints(12, 0)]
#[case::budget_updates(0, 12)]
#[case::mixed(6, 6)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_store_saves_every_queued_write_when_each_lens_read_is_slow(
    #[future(awt)] database: Database,
    active: Lens,
    #[case] checkpoints: u8,
    #[case] settles: u8,
) {
    let holds: Vec<_> = (0..settles)
        .map(|index| BudgetReservation {
            id: format!("request-{index}"),
            job_id: active.jobs[0].id.clone(),
            amount: 1.0,
            month: active.budget_month.clone(),
            expires_at: Some(Utc::now() + BUDGET_LEASE),
        })
        .collect();
    let initial = Lens {
        reservations: holds.clone(),
        ..active
    };
    database.repository().create(&initial).await.unwrap();
    let (_proxy, repository) =
        slow_lens_reads(&database, Duration::from_secs(3), LensRead::Answered).await;
    let saves: Vec<_> = (0..checkpoints)
        .map(|index| {
            let repository = repository.clone();
            let assigned = initial.jobs[0].clone();
            tokio::spawn(async move {
                repository
                    .save_analysis_checkpoint(
                        "lens",
                        &assigned,
                        &format!("fingerprint-{index}"),
                        &grouping(index),
                    )
                    .await
                    .map(value)
                    .map_err(|error| error.to_string())
            })
        })
        .collect();
    let settlements: Vec<_> = holds
        .iter()
        .map(|hold| {
            let repository = repository.clone();
            let id = hold.id.clone();
            tokio::spawn(async move {
                repository
                    .update_locked("lens", move |lens| {
                        Ok::<_, RepositoryError>(settle_amount(lens, &id, 1.0, None))
                    })
                    .await
                    .map(|saved| saved.is_some())
                    .map_err(|error| error.to_string())
            })
        })
        .collect();
    let saved = joined(saves).await;
    let settled = joined(settlements).await;
    let documents: Vec<_> = (0..checkpoints)
        .map(|index| value(grouping(index)))
        .collect();
    assert_eq!(saved, documents.iter().cloned().map(Ok).collect::<Vec<_>>());
    assert_eq!(settled, vec![Ok(true); usize::from(settles)]);
    let keys: Vec<_> = (0..checkpoints)
        .map(|index| {
            record_key(
                "analysis-checkpoint",
                &["lens", &format!("fingerprint-{index}")],
            )
        })
        .collect();
    let references: Vec<_> = keys.iter().map(String::as_str).collect();
    let stored: Vec<_> = database
        .store
        .read_many(&references)
        .await
        .unwrap()
        .into_iter()
        .map(|record| record.value)
        .collect();
    assert_eq!(stored, documents);
    let lens = database.repository().get("lens").await.unwrap().unwrap();
    assert_eq!(lens.spent, initial.spent + f64::from(settles));
    assert_eq!(lens.jobs[0].cost, f64::from(settles));
    assert!(lens.reservations.is_empty());
}
