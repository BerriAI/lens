#[allow(
    dead_code,
    reason = "state fixtures are shared with the state integration suite"
)]
#[path = "state/support.rs"]
mod support;

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use lens_contract::eval::{
    CalledBefore, CaseResult, CreateEvalRun, Gate, GateResult, RunStatus, Scorer, Summary,
    TaskCompleted, TraceRef,
};
use litellm_storage_clickhouse::{
    EvalError,
    evals::{EvalStore, RunCompletion, RunFilter, StoredCase, StoredRun},
};
use rstest::{fixture, rstest};
use support::{Database, database};

#[fixture]
fn request() -> CreateEvalRun {
    CreateEvalRun {
        eval: "regressions".into(),
        agent: "agent".into(),
        dataset_id: "dataset".into(),
        revision: 7,
        case_ids: None,
        version: "sha-candidate".into(),
        branch: "main".into(),
        pr: None,
        ci_url: String::new(),
        trials: 2,
        scorers: vec![
            Scorer::TaskCompleted(TaskCompleted {}),
            Scorer::CalledBefore(CalledBefore {
                first: "read".into(),
                then: "write".into(),
            }),
        ],
        gate: Gate::default(),
        timeout_per_trial_ms: 1_000,
    }
}

#[fixture]
fn cases() -> Vec<StoredCase> {
    vec![
        StoredCase {
            id: "first".into(),
            title: "First case".into(),
            critical: true,
            expected: "done".into(),
        },
        StoredCase {
            id: "second".into(),
            title: "Second case".into(),
            critical: false,
            expected: String::new(),
        },
    ]
}

#[fixture]
fn result() -> CaseResult {
    CaseResult {
        trace: Some(TraceRef {
            attribute: Default::default(),
            value: "session-1".into(),
        }),
        ..CaseResult::default()
    }
}

async fn create(
    store: &EvalStore,
    team: &str,
    request: CreateEvalRun,
    cases: Vec<StoredCase>,
    now: DateTime<Utc>,
) -> StoredRun {
    store
        .create(team, request, cases, None, "http://localhost:4100", now)
        .await
        .unwrap()
}

async fn complete(store: &EvalStore, run: &StoredRun, now: DateTime<Utc>) -> StoredRun {
    let current = store.get(&run.team, &run.run.id).await.unwrap();
    let scoring = if current.run.status == RunStatus::Running {
        store.finish(&run.team, &run.run.id, now).await.unwrap()
    } else {
        current
    };
    let lease = store
        .claim_scoring(&run.team, &run.run.id, now)
        .await
        .unwrap()
        .unwrap();
    let summary = Summary {
        passed: 0,
        total: run.cases.len() as u64,
        pass_rate: 0.0,
        cost_per_case: 0.0,
        scores: BTreeMap::new(),
        errors: run.run.expected_trials,
        baseline_run_id: None,
        baseline_version: None,
        regressions: Vec::new(),
        fixed: Vec::new(),
        gate: GateResult {
            passed: true,
            reasons: Vec::new(),
        },
    };
    store
        .complete(
            &run.team,
            &run.run.id,
            &lease,
            RunCompletion {
                summary,
                trials: scoring.trials,
                verdicts: run
                    .cases
                    .iter()
                    .map(|case| (case.id.clone(), false))
                    .collect(),
            },
            now,
        )
        .await
        .unwrap()
}

#[rstest]
#[tokio::test]
async fn concurrent_create_retries_reuse_one_run_with_team_scoping(
    #[future(awt)] database: Database,
    request: CreateEvalRun,
    cases: Vec<StoredCase>,
) {
    let store = EvalStore::new(database.store.clone());
    let peer = EvalStore::new(database.independent());
    let now = Utc::now();
    let (first, second) = tokio::join!(
        store.create(
            "team",
            request.clone(),
            cases.clone(),
            Some("ci-attempt"),
            "http://localhost:4100",
            now
        ),
        peer.create(
            "team",
            request.clone(),
            cases.clone(),
            Some("ci-attempt"),
            "http://localhost:4100",
            now
        ),
    );
    let first = first.unwrap();
    assert_eq!(first, second.unwrap());
    assert_eq!(
        store.list("team", &RunFilter::default()).await.unwrap(),
        vec![first.clone()]
    );
    let other = store
        .create(
            "other",
            request.clone(),
            cases.clone(),
            Some("ci-attempt"),
            "http://localhost:4100",
            now,
        )
        .await
        .unwrap();
    assert_ne!(first.run.id, other.run.id);
    assert!(matches!(
        store.get("other", &first.run.id).await,
        Err(EvalError::RunNotFound)
    ));
    let changed = CreateEvalRun {
        version: "different".into(),
        ..request
    };
    assert!(matches!(
        store
            .create(
                "team",
                changed,
                cases,
                Some("ci-attempt"),
                "http://localhost:4100",
                now
            )
            .await,
        Err(EvalError::IdempotencyConflict)
    ));
}

#[rstest]
#[tokio::test]
async fn three_identical_puts_store_one_result_without_new_revisions(
    #[future(awt)] database: Database,
    request: CreateEvalRun,
    cases: Vec<StoredCase>,
    result: CaseResult,
) {
    let store = EvalStore::new(database.store.clone());
    let now = Utc::now();
    let run = create(&store, "team", request, cases, now).await;
    store
        .put_result("team", &run.run.id, "second", 1, result.clone(), now)
        .await
        .unwrap();
    let stored = store.get("team", &run.run.id).await.unwrap();
    let rows = database.sql("SELECT count() FROM lens_state_blobs").await;
    let keys = database.store.keys("eval-run/", "", 100).await.unwrap();
    let before = database.store.read(&keys[0]).await.unwrap();
    store
        .put_result(
            "team",
            &run.run.id,
            "second",
            1,
            result.clone(),
            now + Duration::seconds(1),
        )
        .await
        .unwrap();
    store
        .put_result(
            "team",
            &run.run.id,
            "second",
            1,
            result,
            now + Duration::seconds(2),
        )
        .await
        .unwrap();
    assert_eq!(store.get("team", &run.run.id).await.unwrap(), stored);
    assert_eq!(stored.trials.len(), 1);
    assert_eq!(stored.run.received_trials, 1);
    assert_eq!(database.store.read(&keys[0]).await.unwrap(), before);
    assert_eq!(
        database.sql("SELECT count() FROM lens_state_blobs").await,
        rows
    );
}

#[rstest]
#[tokio::test]
async fn replacing_a_result_preserves_the_original_trace_deadline(
    #[future(awt)] database: Database,
    request: CreateEvalRun,
    cases: Vec<StoredCase>,
    result: CaseResult,
) {
    let store = EvalStore::new(database.store.clone());
    let now = Utc::now();
    let run = create(&store, "team", request, cases, now).await;
    store
        .put_result("team", &run.run.id, "first", 0, result.clone(), now)
        .await
        .unwrap();
    let replacement = CaseResult {
        cost_usd: Some(0.25),
        ..result
    };
    store
        .put_result(
            "team",
            &run.run.id,
            "first",
            0,
            replacement.clone(),
            now + Duration::hours(1),
        )
        .await
        .unwrap();
    let updated = store.get("team", &run.run.id).await.unwrap();
    assert_eq!(updated.run.received_trials, 1);
    assert_eq!(updated.trials[0].result, replacement);
    assert_eq!(updated.trials[0].submitted_at, now);
}

#[rstest]
#[tokio::test]
async fn concurrent_puts_preserve_results_and_finish_fences_late_writes(
    #[future(awt)] database: Database,
    request: CreateEvalRun,
    cases: Vec<StoredCase>,
    result: CaseResult,
) {
    let store = EvalStore::new(database.store.clone());
    let peer = EvalStore::new(database.independent());
    let now = Utc::now();
    let run = create(&store, "team", request, cases, now).await;
    let (first, second) = tokio::join!(
        store.put_result("team", &run.run.id, "second", 1, result.clone(), now),
        peer.put_result("team", &run.run.id, "first", 0, result.clone(), now),
    );
    first.unwrap();
    second.unwrap();
    assert_eq!(
        store
            .get("team", &run.run.id)
            .await
            .unwrap()
            .run
            .received_trials,
        2
    );
    let (upload, finish) = tokio::join!(
        store.put_result("team", &run.run.id, "first", 1, result.clone(), now),
        peer.finish("team", &run.run.id, now),
    );
    let finished = finish.unwrap();
    assert_eq!(finished.run.status, RunStatus::Scoring);
    assert_eq!(finished.trials.len() as u64, finished.run.expected_trials);
    let expected_received = match upload {
        Ok(()) => 3,
        Err(EvalError::RunClosed) => 2,
        Err(error) => panic!("{error}"),
    };
    assert_eq!(finished.run.received_trials, expected_received);
    assert_eq!(
        finished
            .trials
            .iter()
            .filter(|trial| trial
                .result
                .error
                .as_ref()
                .is_some_and(|error| error.r#type == "missing_result"))
            .count() as u64,
        finished.run.expected_trials - expected_received
    );
    assert!(matches!(
        store
            .put_result("team", &run.run.id, "second", 0, result, now)
            .await,
        Err(EvalError::RunClosed)
    ));
    assert!(matches!(
        store
            .finish("team", &run.run.id, now + Duration::seconds(1))
            .await,
        Err(EvalError::RunClosed)
    ));
}

#[derive(Clone, Copy)]
enum InvalidSubmission {
    UnknownCase,
    InvalidTrial,
    OtherTeam,
}

#[rstest]
#[case::unknown_case(InvalidSubmission::UnknownCase)]
#[case::invalid_trial(InvalidSubmission::InvalidTrial)]
#[case::other_team(InvalidSubmission::OtherTeam)]
#[tokio::test]
async fn results_validate_membership_without_changing_the_run(
    #[future(awt)] database: Database,
    request: CreateEvalRun,
    cases: Vec<StoredCase>,
    result: CaseResult,
    #[case] submission: InvalidSubmission,
) {
    let store = EvalStore::new(database.store.clone());
    let now = Utc::now();
    let run = create(&store, "team", request, cases, now).await;
    let failure = match submission {
        InvalidSubmission::UnknownCase => {
            store
                .put_result("team", &run.run.id, "missing", 0, result, now)
                .await
        }
        InvalidSubmission::InvalidTrial => {
            store
                .put_result("team", &run.run.id, "first", 2, result, now)
                .await
        }
        InvalidSubmission::OtherTeam => {
            store
                .put_result("other", &run.run.id, "first", 0, result, now)
                .await
        }
    }
    .unwrap_err();
    assert!(matches!(
        (submission, failure),
        (InvalidSubmission::UnknownCase, EvalError::UnknownCase)
            | (InvalidSubmission::InvalidTrial, EvalError::InvalidTrial)
            | (InvalidSubmission::OtherTeam, EvalError::RunNotFound)
    ));
    assert_eq!(store.get("team", &run.run.id).await.unwrap(), run);
}

#[rstest]
#[tokio::test]
async fn restart_recovers_scoring_and_terminal_runs_leave_the_queue(
    #[future(awt)] database: Database,
    request: CreateEvalRun,
    cases: Vec<StoredCase>,
) {
    let store = EvalStore::new(database.store.clone());
    let now = Utc::now();
    let first = create(&store, "team", request.clone(), cases.clone(), now).await;
    let second = create(&store, "team", request, cases, now).await;
    store.finish("team", &first.run.id, now).await.unwrap();
    store.finish("team", &second.run.id, now).await.unwrap();
    let restarted = EvalStore::new(database.independent());
    let page = restarted.scoring("", 1).await.unwrap();
    assert_eq!(page.runs.len(), 1);
    let remaining = restarted
        .scoring(page.next.as_deref().unwrap(), 1)
        .await
        .unwrap();
    assert_eq!(remaining.runs.len(), 1);
    assert_ne!(page.runs[0].run.id, remaining.runs[0].run.id);
    let done = complete(&restarted, &first, now).await;
    let lease = restarted
        .claim_scoring("team", &second.run.id, now)
        .await
        .unwrap()
        .unwrap();
    let failed = restarted
        .fail("team", &second.run.id, &lease, "trace timed out", now)
        .await
        .unwrap();
    assert_eq!(done.run.status, RunStatus::Done);
    assert_eq!(failed.run.status, RunStatus::Failed);
    assert!(restarted.scoring("", 100).await.unwrap().runs.is_empty());
    assert_eq!(
        restarted
            .fail("team", &first.run.id, &lease, "late failure", now)
            .await
            .unwrap(),
        done
    );
}

#[rstest]
#[tokio::test]
async fn baseline_uses_latest_completion_and_order_independent_scorers(
    #[future(awt)] database: Database,
    request: CreateEvalRun,
    cases: Vec<StoredCase>,
) {
    let store = EvalStore::new(database.store.clone());
    let now = Utc::now();
    let old = create(&store, "team", request.clone(), cases.clone(), now).await;
    let newer = create(
        &store,
        "team",
        request.clone(),
        cases.clone(),
        now + Duration::seconds(1),
    )
    .await;
    let early = complete(&store, &newer, now + Duration::seconds(2)).await;
    let latest = complete(&store, &old, now + Duration::seconds(3)).await;
    let candidate_request = CreateEvalRun {
        branch: "pr-feature".into(),
        scorers: request.scorers.into_iter().rev().collect(),
        ..request
    };
    let candidate = create(
        &store,
        "team",
        candidate_request,
        cases,
        now + Duration::seconds(4),
    )
    .await;
    assert_eq!(
        store.baseline(&candidate).await.unwrap(),
        Some(latest.clone())
    );
    assert_eq!(store.baseline(&latest).await.unwrap(), Some(early));
}

#[derive(Clone, Copy)]
enum Mismatch {
    Team,
    Eval,
    Agent,
    Dataset,
    Revision,
    Scorers,
    Branch,
}

#[rstest]
#[tokio::test]
async fn scoring_leases_have_one_owner_and_fence_an_expired_worker(
    #[future(awt)] database: Database,
    request: CreateEvalRun,
    cases: Vec<StoredCase>,
) {
    let store = EvalStore::new(database.store.clone());
    let peer = EvalStore::new(database.independent());
    let now = Utc::now();
    let run = create(&store, "team", request, cases, now).await;
    store.finish("team", &run.run.id, now).await.unwrap();
    let (first, second) = tokio::join!(
        store.claim_scoring("team", &run.run.id, now),
        peer.claim_scoring("team", &run.run.id, now)
    );
    let claims = [first.unwrap(), second.unwrap()];
    assert_eq!(claims.iter().flatten().count(), 1);
    let lease = claims.into_iter().flatten().next().unwrap();
    let renewed = store
        .renew_scoring("team", &run.run.id, &lease, now + Duration::seconds(30))
        .await
        .unwrap();
    assert!(renewed.expires_at > lease.expires_at);
    assert!(
        peer.claim_scoring("team", &run.run.id, now + Duration::seconds(61))
            .await
            .unwrap()
            .is_none()
    );
    let takeover_time = renewed.expires_at + Duration::seconds(1);
    let takeover = peer
        .claim_scoring("team", &run.run.id, takeover_time)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(takeover.token, lease.token);
    assert!(matches!(
        store
            .fail("team", &run.run.id, &lease, "obsolete", takeover_time)
            .await,
        Err(EvalError::LeaseLost)
    ));
    assert!(matches!(
        store.release_scoring("team", &run.run.id, &lease).await,
        Err(EvalError::LeaseLost)
    ));
    let failed = peer
        .fail(
            "team",
            &run.run.id,
            &takeover,
            "current owner",
            takeover_time,
        )
        .await
        .unwrap();
    assert_eq!(failed.run.failure, "current owner");
}

#[rstest]
#[tokio::test]
async fn releasing_a_lease_makes_pending_traces_available_to_another_worker(
    #[future(awt)] database: Database,
    request: CreateEvalRun,
    cases: Vec<StoredCase>,
) {
    let store = EvalStore::new(database.store.clone());
    let now = Utc::now();
    let run = create(&store, "team", request, cases, now).await;
    store.finish("team", &run.run.id, now).await.unwrap();
    let lease = store
        .claim_scoring("team", &run.run.id, now)
        .await
        .unwrap()
        .unwrap();
    store
        .release_scoring("team", &run.run.id, &lease)
        .await
        .unwrap();
    let next = store
        .claim_scoring("team", &run.run.id, now)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(lease.token, next.token);
}

#[rstest]
#[case::team(Mismatch::Team)]
#[case::eval(Mismatch::Eval)]
#[case::agent(Mismatch::Agent)]
#[case::dataset(Mismatch::Dataset)]
#[case::revision(Mismatch::Revision)]
#[case::scorers(Mismatch::Scorers)]
#[case::branch(Mismatch::Branch)]
#[tokio::test]
async fn baseline_excludes_incompatible_runs(
    #[future(awt)] database: Database,
    request: CreateEvalRun,
    cases: Vec<StoredCase>,
    #[case] mismatch: Mismatch,
) {
    let store = EvalStore::new(database.store.clone());
    let now = Utc::now();
    let incompatible = match mismatch {
        Mismatch::Eval => CreateEvalRun {
            eval: "other".into(),
            ..request.clone()
        },
        Mismatch::Agent => CreateEvalRun {
            agent: "other".into(),
            ..request.clone()
        },
        Mismatch::Dataset => CreateEvalRun {
            dataset_id: "other".into(),
            ..request.clone()
        },
        Mismatch::Revision => CreateEvalRun {
            revision: 8,
            ..request.clone()
        },
        Mismatch::Scorers => CreateEvalRun {
            scorers: vec![Scorer::TaskCompleted(TaskCompleted {})],
            ..request.clone()
        },
        Mismatch::Branch => CreateEvalRun {
            branch: "feature".into(),
            ..request.clone()
        },
        Mismatch::Team => request.clone(),
    };
    let team = if matches!(mismatch, Mismatch::Team) {
        "other"
    } else {
        "team"
    };
    let baseline = create(&store, team, incompatible, cases.clone(), now).await;
    complete(&store, &baseline, now).await;
    let candidate = create(&store, "team", request, cases, now).await;
    assert_eq!(store.baseline(&candidate).await.unwrap(), None);
}

#[rstest]
#[tokio::test]
async fn list_applies_all_filters_and_limit_without_crossing_teams(
    #[future(awt)] database: Database,
    request: CreateEvalRun,
    cases: Vec<StoredCase>,
) {
    let store = EvalStore::new(database.store.clone());
    let now = Utc::now();
    let older = create(&store, "team", request.clone(), cases.clone(), now).await;
    let newest = create(
        &store,
        "team",
        request.clone(),
        cases.clone(),
        now + Duration::seconds(1),
    )
    .await;
    create(&store, "other", request, cases, now + Duration::seconds(2)).await;
    let filter = RunFilter {
        eval: Some(older.request.eval.clone()),
        agent: Some(older.request.agent.clone()),
        branch: Some("main".into()),
        dataset: Some(older.request.dataset_id.clone()),
        after: None,
        limit: 1,
    };
    assert_eq!(
        store.list("team", &filter).await.unwrap(),
        vec![newest.clone()]
    );
    assert_eq!(
        store
            .list(
                "team",
                &RunFilter {
                    after: Some(newest.run.id.clone()),
                    ..filter.clone()
                }
            )
            .await
            .unwrap(),
        vec![older]
    );
    assert!(
        store
            .list(
                "team",
                &RunFilter {
                    dataset: Some("missing".into()),
                    ..filter.clone()
                }
            )
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .list(
                "team",
                &RunFilter {
                    branch: Some("missing".into()),
                    ..filter
                }
            )
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        store
            .list(
                "team",
                &RunFilter {
                    limit: 101,
                    ..RunFilter::default()
                }
            )
            .await,
        Err(EvalError::InvalidRequest(_))
    ));
}
