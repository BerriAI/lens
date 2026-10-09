use super::support::isolated_database;
use super::{qualification_support::*, *};
use litellm_storage_clickhouse::state::Change;

#[rstest]
#[case::legacy(None)]
#[case::stale(Some("https://github.com/old/repo/actions/runs/1"))]
#[tokio::test]
async fn stored_request_preserves_ci_provenance_for_existing_runs(
    #[future(awt)] live_run: LiveRun,
    #[case] saved_url: Option<&str>,
) {
    let ci_url = "https://github.com/example/agent/actions/runs/42";
    let mut saved = serde_json::to_value(&live_run.run).unwrap();
    saved["request"]["ci_url"] = ci_url.into();
    let run = saved["run"].as_object_mut().unwrap();
    match saved_url {
        Some(url) => run.insert("ci_url".into(), url.into()),
        None => run.remove("ci_url"),
    };
    live_run.replace(saved).await;
    let restored = live_run
        .store
        .get("team", &live_run.run.run.id)
        .await
        .unwrap();
    assert_eq!(restored.request.ci_url, ci_url);
    assert!(restored.run.ci_url.is_empty());
    assert_eq!(
        live_run
            .store
            .list("team", &RunFilter::default())
            .await
            .unwrap(),
        vec![restored]
    );
}

#[rstest]
#[tokio::test]
async fn ci_metadata_keeps_the_stored_run_compatible_with_older_readers(
    #[future(awt)] database: Database,
    request: CreateEvalRun,
    cases: Vec<StoredCase>,
) {
    let ci_url = "https://github.com/example/agent/actions/runs/42";
    let store = EvalStore::new(database.store.clone());
    let run = create(
        &store,
        "team",
        CreateEvalRun {
            ci_url: ci_url.into(),
            ..request
        },
        cases,
        Utc::now(),
    )
    .await;
    let key = database.store.keys("eval-run/", "", 100).await.unwrap();
    let snapshot = database.store.read(&key[0]).await.unwrap();
    assert_eq!(snapshot.value["request"]["ci_url"], ci_url);
    assert!(snapshot.value["run"].get("ci_url").is_none());
    assert!(run.run.ci_url.is_empty());
}

#[rstest]
#[case::run("eval-run/")]
#[case::idempotency("eval-idempotency/")]
#[tokio::test]
async fn failed_creation_publishes_neither_run_nor_idempotency(
    #[future(awt)] database: Database,
    request: CreateEvalRun,
    cases: Vec<StoredCase>,
    #[case] prefix: &str,
) {
    let store = EvalStore::new(database.independent());
    reject_writes(&database, prefix).await;
    let result = store
        .create(
            "team",
            request,
            cases,
            Some("retry"),
            "http://lens.local",
            Utc::now(),
        )
        .await;
    assert!(storage_failure(&result.unwrap_err()));
    assert_eq!(
        database
            .sql("SELECT count() FROM lens_state_heads WHERE revision > 0")
            .await
            .trim(),
        "0"
    );
    assert!(
        store
            .list("team", &RunFilter::default())
            .await
            .unwrap()
            .is_empty()
    );
}

#[rstest]
#[tokio::test]
async fn failed_result_publication_keeps_original_receipt(
    #[future(awt)] live_run: LiveRun,
    result: CaseResult,
) {
    let before = live_run.snapshot().await;
    reject_writes(&live_run.database, "eval-run/").await;
    let error = live_run
        .store
        .put_result(
            "team",
            &live_run.run.run.id,
            "first",
            0,
            result,
            live_run.now,
        )
        .await
        .unwrap_err();
    assert!(storage_failure(&error));
    assert_eq!(live_run.snapshot().await, before);
    assert_eq!(
        live_run
            .store
            .get("team", &live_run.run.run.id)
            .await
            .unwrap(),
        live_run.run
    );
}

#[rstest]
#[tokio::test]
async fn unpublished_future_run_never_replaces_published_result(#[future(awt)] live_run: LiveRun) {
    let candidate = StoredRun {
        run: lens_contract::eval::EvalRun {
            failure: "unpublished".into(),
            ..live_run.run.run.clone()
        },
        ..live_run.run.clone()
    };
    live_run
        .database
        .store
        .prepare(vec![Change {
            previous: live_run.snapshot().await,
            value: serde_json::to_value(candidate).unwrap(),
        }])
        .await
        .unwrap();
    assert_eq!(
        live_run
            .store
            .get("team", &live_run.run.run.id)
            .await
            .unwrap(),
        live_run.run
    );
    assert_eq!(
        live_run
            .store
            .list("team", &RunFilter::default())
            .await
            .unwrap(),
        vec![live_run.run]
    );
}

#[rstest]
#[case::get(false)]
#[case::list(true)]
#[tokio::test]
async fn array_encoded_run_records_fail_closed(
    #[future(awt)] live_run: LiveRun,
    #[case] list: bool,
) {
    live_run.replace(run_as_array(&live_run.run)).await;
    let error = if list {
        live_run
            .store
            .list("team", &RunFilter::default())
            .await
            .unwrap_err()
    } else {
        live_run
            .store
            .get("team", &live_run.run.run.id)
            .await
            .unwrap_err()
    };
    assert!(invalid_state(&error));
}

#[rstest]
#[tokio::test]
async fn listing_crosses_full_pages_and_skips_deleted_records(#[future(awt)] paged_runs: LiveRun) {
    let selected = paged_runs
        .store
        .list(
            "team",
            &RunFilter {
                agent: Some("needle".into()),
                ..RunFilter::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].run.id, "batch-129");
    let first = paged_runs
        .store
        .list(
            "team",
            &RunFilter {
                limit: 100,
                ..RunFilter::default()
            },
        )
        .await
        .unwrap();
    let second = paged_runs
        .store
        .list(
            "team",
            &RunFilter {
                after: Some(first.last().unwrap().run.id.clone()),
                limit: 100,
                ..RunFilter::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(first.len(), 100);
    assert_eq!(second.len(), 31);
    assert!(second[0].run.id > first.last().unwrap().run.id);
}

#[rstest]
#[case::before(-1, false)]
#[case::exact(0, true)]
#[case::after(1, true)]
#[tokio::test]
async fn scoring_claim_recovery_obeys_expiration_boundary(
    #[future(awt)] live_run: LiveRun,
    #[case] offset: i64,
    #[case] takeover: bool,
) {
    let (_, lease) = live_run.scoring().await;
    let next = live_run
        .store
        .claim_scoring(
            "team",
            &live_run.run.run.id,
            lease.expires_at + Duration::seconds(offset),
        )
        .await
        .unwrap();
    assert_eq!(next.is_some(), takeover);
    if let Some(next) = next {
        assert_ne!(next.token, lease.token);
    }
}

#[rstest]
#[tokio::test]
async fn delayed_renewal_cannot_shorten_the_current_lease(#[future(awt)] live_run: LiveRun) {
    let (_, lease) = live_run.scoring().await;
    let renewed = live_run
        .store
        .renew_scoring(
            "team",
            &live_run.run.run.id,
            &lease,
            live_run.now + Duration::seconds(30),
        )
        .await
        .unwrap();
    let before = live_run.snapshot().await;
    let late = live_run
        .store
        .renew_scoring(
            "team",
            &live_run.run.run.id,
            &lease,
            live_run.now + Duration::seconds(1),
        )
        .await
        .unwrap();
    assert_eq!(late, renewed);
    assert_eq!(live_run.snapshot().await, before);
}

#[derive(Clone, Copy)]
enum TerminalAction {
    Complete,
    Fail,
    Renew,
    Release,
}

#[rstest]
#[case::complete(TerminalAction::Complete)]
#[case::fail(TerminalAction::Fail)]
#[case::renew(TerminalAction::Renew)]
#[case::release(TerminalAction::Release)]
#[tokio::test]
async fn replaced_owner_cannot_change_or_release_successor(
    #[future(awt)] live_run: LiveRun,
    #[case] action: TerminalAction,
) {
    let (run, lease) = live_run.scoring().await;
    let now = lease.expires_at;
    let successor = live_run
        .store
        .claim_scoring("team", &run.run.id, now)
        .await
        .unwrap()
        .unwrap();
    let before = live_run.snapshot().await;
    let error = match action {
        TerminalAction::Complete => live_run
            .store
            .complete("team", &run.run.id, &lease, completion(&run), now)
            .await
            .map(|_| ()),
        TerminalAction::Fail => live_run
            .store
            .fail("team", &run.run.id, &lease, "stale", now)
            .await
            .map(|_| ()),
        TerminalAction::Renew => live_run
            .store
            .renew_scoring("team", &run.run.id, &lease, now)
            .await
            .map(|_| ()),
        TerminalAction::Release => {
            live_run
                .store
                .release_scoring("team", &run.run.id, &lease)
                .await
        }
    }
    .unwrap_err();
    assert!(matches!(error, EvalError::LeaseLost));
    assert_eq!(live_run.snapshot().await, before);
    assert_eq!(
        live_run
            .store
            .get("team", &run.run.id)
            .await
            .unwrap()
            .scoring_lease,
        Some(successor)
    );
}

#[rstest]
#[case::exact(0)]
#[case::after(1)]
#[tokio::test]
async fn expired_owner_cannot_renew_without_a_takeover(
    #[future(awt)] live_run: LiveRun,
    #[case] offset: i64,
) {
    let (_, lease) = live_run.scoring().await;
    let before = live_run.snapshot().await;
    assert!(matches!(
        live_run
            .store
            .renew_scoring(
                "team",
                &live_run.run.run.id,
                &lease,
                lease.expires_at + Duration::seconds(offset)
            )
            .await,
        Err(EvalError::LeaseLost)
    ));
    assert_eq!(live_run.snapshot().await, before);
}

#[derive(Clone, Copy)]
enum InvalidCompletion {
    SubmissionTime,
    InvalidResult,
    MissingTrial,
    DuplicateTrial,
    ForeignVerdict,
    Total,
}

#[rstest]
#[case::submission_time(InvalidCompletion::SubmissionTime)]
#[case::invalid_result(InvalidCompletion::InvalidResult)]
#[case::missing_trial(InvalidCompletion::MissingTrial)]
#[case::duplicate_trial(InvalidCompletion::DuplicateTrial)]
#[case::foreign_verdict(InvalidCompletion::ForeignVerdict)]
#[case::total(InvalidCompletion::Total)]
#[tokio::test]
async fn invalid_completion_cannot_replace_sealed_results(
    #[future(awt)] live_run: LiveRun,
    #[case] invalid: InvalidCompletion,
) {
    let (run, lease) = live_run.scoring().await;
    let mut completion = completion(&run);
    match invalid {
        InvalidCompletion::SubmissionTime => {
            completion.trials[0].submitted_at += Duration::seconds(1)
        }
        InvalidCompletion::InvalidResult => completion.trials[0].result = CaseResult::default(),
        InvalidCompletion::MissingTrial => {
            completion.trials.pop();
        }
        InvalidCompletion::DuplicateTrial => completion.trials.push(completion.trials[0].clone()),
        InvalidCompletion::ForeignVerdict => {
            completion.verdicts.insert("foreign".into(), true);
        }
        InvalidCompletion::Total => completion.summary.total += 1,
    }
    let before = live_run.snapshot().await;
    assert!(matches!(
        live_run
            .store
            .complete("team", &run.run.id, &lease, completion, live_run.now)
            .await,
        Err(EvalError::InvalidRequest(_))
    ));
    assert_eq!(live_run.snapshot().await, before);
    assert_eq!(live_run.store.scoring("", 100).await.unwrap().runs.len(), 1);
}

#[rstest]
#[case::empty(0)]
#[case::too_large(101)]
#[tokio::test]
async fn invalid_page_sizes_fail_before_reading_runs(
    #[future(awt)] live_run: LiveRun,
    #[case] limit: u32,
) {
    assert!(matches!(
        live_run
            .store
            .list(
                "team",
                &RunFilter {
                    limit,
                    ..RunFilter::default()
                }
            )
            .await,
        Err(EvalError::InvalidRequest(_))
    ));
    assert!(matches!(
        live_run.store.scoring("", limit).await,
        Err(EvalError::InvalidRequest(_))
    ));
}

#[rstest]
#[tokio::test]
async fn successful_completion_persists_enriched_results_with_original_times(
    #[future(awt)] live_run: LiveRun,
    result: CaseResult,
) {
    live_run
        .store
        .put_result(
            "team",
            &live_run.run.run.id,
            "first",
            0,
            result,
            live_run.now,
        )
        .await
        .unwrap();
    let (run, lease) = live_run.scoring().await;
    let mut completion = completion(&run);
    completion.trials[0].result.cost_usd = Some(0.75);
    let expected = completion.trials.clone();
    let done = live_run
        .store
        .complete("team", &run.run.id, &lease, completion, live_run.now)
        .await
        .unwrap();
    let restarted = EvalStore::new(live_run.database.independent());
    assert_eq!(done.run.status, RunStatus::Done);
    assert_eq!(done.trials, expected);
    assert_eq!(restarted.get("team", &run.run.id).await.unwrap(), done);
    assert!(restarted.scoring("", 100).await.unwrap().runs.is_empty());
}

#[rstest]
#[tokio::test]
async fn missing_run_operations_return_not_found(
    #[future(awt)] database: Database,
    result: CaseResult,
) {
    let store = EvalStore::new(database.independent());
    assert!(matches!(
        store
            .put_result("team", "missing", "first", 0, result, Utc::now())
            .await,
        Err(EvalError::RunNotFound)
    ));
    assert!(matches!(
        store.finish("team", "missing", Utc::now()).await,
        Err(EvalError::RunNotFound)
    ));
    assert!(matches!(
        store.claim_scoring("team", "missing", Utc::now()).await,
        Err(EvalError::RunNotFound)
    ));
    assert_eq!(
        database
            .sql("SELECT count() FROM lens_state_blobs")
            .await
            .trim(),
        "0"
    );
}

#[rstest]
#[tokio::test]
async fn run_and_first_receipt_survive_abrupt_restart(
    #[future(awt)] isolated_database: Database,
    request: CreateEvalRun,
    cases: Vec<StoredCase>,
    result: CaseResult,
) {
    let database = isolated_database;
    let store = EvalStore::new(database.independent());
    let now = Utc::now();
    let run = create(&store, "team", request, cases, now).await;
    store
        .put_result("team", &run.run.id, "first", 0, result.clone(), now)
        .await
        .unwrap();
    let saved = store.get("team", &run.run.id).await.unwrap();
    let restarted = EvalStore::new(database.restart().await);
    assert_eq!(restarted.get("team", &run.run.id).await.unwrap(), saved);
    restarted
        .put_result(
            "team",
            &run.run.id,
            "first",
            0,
            CaseResult {
                cost_usd: Some(0.5),
                ..result
            },
            now + Duration::seconds(10),
        )
        .await
        .unwrap();
    let updated = restarted.get("team", &run.run.id).await.unwrap();
    assert_eq!(updated.run.received_trials, 1);
    assert_eq!(updated.trials[0].submitted_at, now);
    assert_eq!(updated.trials[0].result.cost_usd, Some(0.5));
}

#[derive(Clone, Copy)]
enum InvalidCreate {
    EmptyKey,
    LongKey,
    Cases,
    DuplicateCase,
    EmptyCase,
    ZeroTrials,
    TooManyTrials,
}

#[rstest]
#[case::empty_key(InvalidCreate::EmptyKey)]
#[case::long_key(InvalidCreate::LongKey)]
#[case::cases(InvalidCreate::Cases)]
#[case::duplicate_case(InvalidCreate::DuplicateCase)]
#[case::empty_case(InvalidCreate::EmptyCase)]
#[case::zero_trials(InvalidCreate::ZeroTrials)]
#[case::too_many_trials(InvalidCreate::TooManyTrials)]
#[tokio::test]
async fn invalid_creation_never_writes_a_record(
    #[future(awt)] database: Database,
    request: CreateEvalRun,
    cases: Vec<StoredCase>,
    #[case] invalid: InvalidCreate,
) {
    let store = EvalStore::new(database.independent());
    let (team, key, request, cases) = match invalid {
        InvalidCreate::EmptyKey => ("team", String::new(), request, cases),
        InvalidCreate::LongKey => ("team", "x".repeat(513), request, cases),
        InvalidCreate::Cases => ("team", "key".to_owned(), request, vec![]),
        InvalidCreate::DuplicateCase => (
            "team",
            "key".to_owned(),
            request,
            vec![cases[0].clone(), cases[0].clone()],
        ),
        InvalidCreate::EmptyCase => (
            "team",
            "key".to_owned(),
            request,
            vec![StoredCase {
                id: String::new(),
                ..cases[0].clone()
            }],
        ),
        InvalidCreate::ZeroTrials => (
            "team",
            "key".to_owned(),
            CreateEvalRun {
                trials: 0,
                ..request
            },
            cases,
        ),
        InvalidCreate::TooManyTrials => (
            "team",
            "key".to_owned(),
            CreateEvalRun {
                trials: 11,
                ..request
            },
            cases,
        ),
    };
    assert!(matches!(
        store
            .create(
                team,
                request,
                cases,
                Some(&key),
                "http://lens.local",
                Utc::now()
            )
            .await,
        Err(EvalError::InvalidRequest(_))
    ));
    assert_eq!(
        database
            .sql("SELECT count() FROM lens_state_blobs")
            .await
            .trim(),
        "0"
    );
    assert_eq!(
        database
            .sql("SELECT count() FROM lens_state_heads")
            .await
            .trim(),
        "0"
    );
}

#[rstest]
#[tokio::test]
async fn standalone_default_scope_remains_isolated_from_named_teams(
    #[future(awt)] database: Database,
    request: CreateEvalRun,
    cases: Vec<StoredCase>,
) {
    let store = EvalStore::new(database.independent());
    let now = Utc::now();
    let spec = lens_contract::eval::EvalSpec {
        agent: request.agent.clone(),
        dataset_id: request.dataset_id.clone(),
        revision: Some(request.revision),
        scorers: request.scorers.clone(),
        trials: request.trials,
        baseline: "main".into(),
        gate: request.gate.clone(),
        timeout_per_trial_ms: request.timeout_per_trial_ms,
    };
    let definition = store
        .put_definition("", "default-eval", spec, now)
        .await
        .unwrap();
    let run = create(&store, "", request, cases, now).await;
    assert_eq!(store.get("", &run.run.id).await.unwrap(), run);
    assert_eq!(
        store.list("", &RunFilter::default()).await.unwrap(),
        vec![run.clone()]
    );
    assert_eq!(
        store.definition("", "default-eval").await.unwrap(),
        definition
    );
    let generated = lens_contract::eval::EvalDefinition {
        name: run.run.eval.clone(),
        ..definition.clone()
    };
    assert_eq!(
        store.definitions("").await.unwrap(),
        vec![definition, generated]
    );
    assert!(matches!(
        store.get("team-a", &run.run.id).await,
        Err(EvalError::RunNotFound)
    ));
    assert!(matches!(
        store.definition("team-a", "default-eval").await,
        Err(EvalError::EvalNotFound)
    ));
    assert!(
        store
            .list("team-a", &RunFilter::default())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(store.definitions("team-a").await.unwrap().is_empty());
}
