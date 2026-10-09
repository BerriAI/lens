#[path = "evals/support.rs"]
mod support;

use axum::http::HeaderValue;
use chrono::Utc;
use lens_contract::eval::{ApiError, ApiErrorCode, EvalRun, RunStatus};
use rstest::rstest;
use serde_json::{Value, json};
use support::{EvalFixture, body, create, create_payload, eval_fixture, guarded_app, request};
use tower::ServiceExt;

#[rstest]
#[tokio::test]
async fn should_create_once_per_team_and_snapshot_included_cases(
    #[future(awt)] eval_fixture: EvalFixture,
) {
    let mut first = request("POST", "/lens/evals/runs", "team-a", Some(create_payload()));
    first.headers_mut().insert(
        "idempotency-key",
        HeaderValue::from_static("regressions:abc123:1"),
    );
    let mut retry = request("POST", "/lens/evals/runs", "team-a", Some(create_payload()));
    retry.headers_mut().insert(
        "idempotency-key",
        HeaderValue::from_static("regressions:abc123:1"),
    );
    let first = eval_fixture.app.clone().oneshot(first).await.unwrap();
    let retry = eval_fixture.app.clone().oneshot(retry).await.unwrap();
    assert_eq!(first.status(), 201);
    assert_eq!(retry.status(), 201);
    let first: EvalRun = body(first).await;
    let retry: EvalRun = body(retry).await;
    assert_eq!(retry, first);
    assert_eq!(first.expected_trials, 6);
    let stored = eval_fixture.store.get("team-a", &first.id).await.unwrap();
    assert_eq!(stored.cases.len(), 2);
    assert!(stored.cases[0].critical);
    assert_eq!(stored.cases[0].title, "Run tests");
}

#[rstest]
#[tokio::test]
async fn should_store_three_identical_puts_once_and_accept_out_of_order_results(
    #[future(awt)] eval_fixture: EvalFixture,
) {
    let run = create(&eval_fixture).await;
    let path = format!("/lens/evals/runs/{}/results/case-2/2", run.id);
    let result = json!({"trace":{"attribute":"session.id","value":"late-trial"}});
    let first = eval_fixture
        .app
        .clone()
        .oneshot(request("PUT", &path, "team-a", Some(result.clone())))
        .await
        .unwrap();
    let second = eval_fixture
        .app
        .clone()
        .oneshot(request("PUT", &path, "team-a", Some(result.clone())))
        .await
        .unwrap();
    let third = eval_fixture
        .app
        .clone()
        .oneshot(request("PUT", &path, "team-a", Some(result)))
        .await
        .unwrap();
    assert_eq!(first.status(), 204);
    assert_eq!(second.status(), 204);
    assert_eq!(third.status(), 204);
    let stored = eval_fixture.store.get("team-a", &run.id).await.unwrap();
    assert_eq!(stored.trials.len(), 1);
    assert_eq!(stored.run.received_trials, 1);
    assert_eq!(stored.trials[0].trial, 2);
}

#[rstest]
#[case::put("PUT", "/results/case-1/0", Some(json!({"error":{"type":"Error","message":"failed"}})))]
#[case::finish("POST", "/finish", None)]
#[tokio::test]
async fn should_close_runs_on_finish_and_fill_missing_trials(
    #[future(awt)] eval_fixture: EvalFixture,
    #[case] method: &str,
    #[case] suffix: &str,
    #[case] payload: Option<Value>,
) {
    let run = create(&eval_fixture).await;
    let finish = eval_fixture
        .app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/lens/evals/runs/{}/finish", run.id),
            "team-a",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(finish.status(), 202);
    let finished: EvalRun = body(finish).await;
    assert_eq!(finished.status, RunStatus::Scoring);
    let stored = eval_fixture.store.get("team-a", &run.id).await.unwrap();
    assert_eq!(stored.trials.len(), 6);
    assert_eq!(
        stored
            .trials
            .iter()
            .filter(|trial| trial.result.error.is_some())
            .count(),
        6
    );
    let response = eval_fixture
        .app
        .clone()
        .oneshot(request(
            method,
            &format!("/lens/evals/runs/{}{suffix}", run.id),
            "team-a",
            payload,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 409);
    assert_eq!(
        body::<ApiError>(response).await.code,
        ApiErrorCode::RunClosed
    );
}

#[rstest]
#[case::unknown("unknown")]
#[case::excluded("excluded")]
#[tokio::test]
async fn should_reject_cases_outside_the_run(
    #[future(awt)] eval_fixture: EvalFixture,
    #[case] case_id: &str,
) {
    let run = create(&eval_fixture).await;
    let response = eval_fixture
        .app
        .clone()
        .oneshot(request(
            "PUT",
            &format!("/lens/evals/runs/{}/results/{case_id}/0", run.id),
            "team-a",
            Some(json!({"error":{"type":"Error","message":"failed"}})),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    assert_eq!(
        body::<ApiError>(response).await.code,
        ApiErrorCode::UnknownCase
    );
}

#[rstest]
#[case::get("GET", "", None)]
#[case::finish("POST", "/finish", None)]
#[case::put("PUT", "/results/case-1/0", Some(json!({"error":{"type":"Error","message":"failed"}})))]
#[tokio::test]
async fn should_hide_other_teams_runs(
    #[future(awt)] eval_fixture: EvalFixture,
    #[case] method: &str,
    #[case] suffix: &str,
    #[case] payload: Option<Value>,
) {
    let run = create(&eval_fixture).await;
    let response = eval_fixture
        .app
        .clone()
        .oneshot(request(
            method,
            &format!("/lens/evals/runs/{}{suffix}", run.id),
            "team-b",
            payload,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    assert_eq!(
        body::<ApiError>(response).await.code,
        ApiErrorCode::RunNotFound
    );
}

#[rstest]
#[case::create("POST", "/lens/evals/runs")]
#[case::put("PUT", "/lens/evals/runs/unknown/results/case-1/0")]
#[case::finish("POST", "/lens/evals/runs/unknown/finish")]
#[case::get("GET", "/lens/evals/runs/unknown")]
#[case::list("GET", "/lens/evals/runs")]
#[case::resolve("GET", "/lens/datasets/resolve?name=regressions")]
#[case::cases("GET", "/lens/datasets/dataset-1/revisions/7/cases")]
#[tokio::test]
async fn should_check_contract_version_on_every_route(
    guarded_app: axum::Router,
    #[case] method: &str,
    #[case] path: &str,
    #[values(None, Some("2"), Some("1.5"))] version: Option<&str>,
) {
    let mut request = request(method, path, "team-a", None);
    request.headers_mut().remove("X-Lens-Contract");
    if let Some(version) = version {
        request
            .headers_mut()
            .insert("X-Lens-Contract", HeaderValue::from_str(version).unwrap());
    }
    let response = guarded_app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), 409);
    assert_eq!(
        body::<ApiError>(response).await.code,
        ApiErrorCode::ContractVersion
    );
}

#[rstest]
#[case::missing(None)]
#[case::forged(Some("Bearer wrong"))]
#[case::teamless(Some("teamless"))]
#[tokio::test]
async fn should_require_valid_authenticated_team_credentials(
    guarded_app: axum::Router,
    #[case] credential: Option<&str>,
) {
    let mut request = request("GET", "/lens/evals/runs", "team-a", None);
    request.headers_mut().remove("authorization");
    if let Some(credential) = credential {
        let credential = if credential == "teamless" {
            format!("Bearer {}", support::token(None))
        } else {
            credential.to_owned()
        };
        request
            .headers_mut()
            .insert("authorization", HeaderValue::from_str(&credential).unwrap());
    }
    let response = guarded_app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), 401);
    assert_eq!(
        body::<ApiError>(response).await.code,
        ApiErrorCode::Unauthorized
    );
}

#[rstest]
#[case::same_team("team-a", "?eval=regressions&agent=agent&branch=main&limit=1", 1)]
#[case::other_team("team-b", "", 0)]
#[case::different_eval("team-a", "?eval=missing", 0)]
#[case::different_agent("team-a", "?agent=missing", 0)]
#[case::different_branch("team-a", "?branch=missing", 0)]
#[tokio::test]
async fn should_filter_and_scope_listed_runs(
    #[future(awt)] eval_fixture: EvalFixture,
    #[case] team: &str,
    #[case] query: &str,
    #[case] count: usize,
) {
    let created = create(&eval_fixture).await;
    let response = eval_fixture
        .app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/lens/evals/runs{query}"),
            team,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let listed: Vec<EvalRun> = body(response).await;
    assert_eq!(listed.len(), count);
    if let Some(run) = listed.first() {
        assert_eq!(run.id, created.id);
    }
}

#[rstest]
#[tokio::test]
async fn should_long_poll_until_terminal_status(#[future(awt)] eval_fixture: EvalFixture) {
    let run = create(&eval_fixture).await;
    eval_fixture
        .store
        .finish("team-a", &run.id, Utc::now())
        .await
        .unwrap();
    let store = eval_fixture.store.clone();
    let id = run.id.clone();
    let complete = async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let lease = store
            .claim_scoring("team-a", &id, Utc::now())
            .await
            .unwrap()
            .unwrap();
        store
            .fail("team-a", &id, &lease, "scoring fixture failure", Utc::now())
            .await
            .unwrap();
    };
    let read = eval_fixture.app.clone().oneshot(request(
        "GET",
        &format!("/lens/evals/runs/{}?wait=30", run.id),
        "team-a",
        None,
    ));
    let (response, ()) = tokio::join!(read, complete);
    let response = response.unwrap();
    assert_eq!(response.status(), 200);
    let result: EvalRun = body(response).await;
    assert_eq!(result.status, RunStatus::Failed);
    assert_eq!(result.failure, "scoring fixture failure");
}

#[rstest]
#[case::latest("?name=regressions")]
#[case::pinned("?name=regressions&revision=7")]
#[tokio::test]
async fn should_resolve_dataset_names_and_return_only_included_cases(
    #[future(awt)] eval_fixture: EvalFixture,
    #[case] query: &str,
) {
    let resolved = eval_fixture
        .app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/lens/datasets/resolve{query}"),
            "team-a",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(resolved.status(), 200);
    assert_eq!(
        body::<Value>(resolved).await,
        json!({"id":"dataset-1","name":"regressions","revision":7})
    );
    let cases = eval_fixture
        .app
        .clone()
        .oneshot(request(
            "GET",
            "/lens/datasets/dataset-1/revisions/7/cases",
            "team-a",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(cases.status(), 200);
    let cases: Value = body(cases).await;
    assert_eq!(cases["cases"].as_array().unwrap().len(), 2);
    assert_eq!(
        cases["cases"][0]["meta"],
        json!({"repo_url":"https://example.test/repo"})
    );
}

#[rstest]
#[case::not_found("team-a", "?name=absent", ApiErrorCode::DatasetNotFound)]
#[case::other_team("team-b", "?name=regressions", ApiErrorCode::DatasetNotFound)]
#[case::unknown_revision(
    "team-a",
    "?name=regressions&revision=8",
    ApiErrorCode::RevisionNotFound
)]
#[tokio::test]
async fn should_return_scoped_dataset_errors(
    #[future(awt)] eval_fixture: EvalFixture,
    #[case] team: &str,
    #[case] query: &str,
    #[case] code: ApiErrorCode,
) {
    let response = eval_fixture
        .app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/lens/datasets/resolve{query}"),
            team,
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    assert_eq!(body::<ApiError>(response).await.code, code);
}

struct NoTraces;

impl lens_server::eval_closer::TraceSource for NoTraces {
    async fn trace(
        &self,
        _team: &str,
        _reference: &lens_contract::eval::TraceRef,
    ) -> Result<Option<litellm_traces_clickhouse::evals::EvalTrace>, lens_server::EvalCloserError>
    {
        Ok(None)
    }
}

struct ErrorScorer;

impl lens_server::eval_closer::ScoreRun for ErrorScorer {
    async fn score(
        &self,
        input: &lens_server::eval_closer::RunScoreInput,
    ) -> Result<lens_server::eval_closer::ScoredRun, lens_server::EvalCloserError> {
        Ok(lens_server::eval_closer::ScoredRun {
            summary: lens_contract::eval::Summary {
                passed: 0,
                total: input.run.cases.len() as u64,
                pass_rate: 0.0,
                cost_per_case: 0.0,
                scores: Default::default(),
                errors: input
                    .trials
                    .iter()
                    .filter(|trial| trial.stored.result.error.is_some())
                    .count() as u64,
                baseline_run_id: None,
                baseline_version: None,
                regressions: Vec::new(),
                fixed: Vec::new(),
                gate: lens_contract::eval::GateResult {
                    passed: false,
                    reasons: vec!["trials failed".into()],
                },
            },
            verdicts: input
                .run
                .cases
                .iter()
                .map(|case| (case.id.clone(), false))
                .collect(),
        })
    }
}

#[rstest]
#[tokio::test]
async fn should_persist_missing_traces_as_errors_before_publishing_summary(
    #[future(awt)] eval_fixture: EvalFixture,
) {
    let run = create(&eval_fixture).await;
    let submitted = eval_fixture
        .app
        .clone()
        .oneshot(request(
            "PUT",
            &format!("/lens/evals/runs/{}/results/case-1/0", run.id),
            "team-a",
            Some(json!({"trace":{"value":"never-arrives"}})),
        ))
        .await
        .unwrap();
    assert_eq!(submitted.status(), 204);
    let finished = eval_fixture
        .app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/lens/evals/runs/{}/finish", run.id),
            "team-a",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(finished.status(), 202);
    let stored = eval_fixture.store.get("team-a", &run.id).await.unwrap();
    lens_server::eval_closer::EvalCloser::new(eval_fixture.store.clone(), NoTraces, ErrorScorer)
        .advance(&stored, Utc::now() + chrono::Duration::seconds(1))
        .await
        .unwrap();
    let response = eval_fixture
        .app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/lens/evals/runs/{}?wait=30", run.id),
            "team-a",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let done: EvalRun = body(response).await;
    assert_eq!(done.status, RunStatus::Done);
    let summary = done.summary.unwrap();
    assert_eq!(summary.errors, 6);
    assert_eq!(summary.passed, 0);
    assert!(!summary.gate.passed);
    let saved = eval_fixture.store.get("team-a", &run.id).await.unwrap();
    assert!(saved.trials[0].result.error.is_some());
    assert!(saved.trials[0].result.trace.is_none());
}

#[rstest]
#[case::zero_trials("trials", json!(0))]
#[case::too_many_trials("trials", json!(11))]
#[case::empty_agent("agent", json!(""))]
#[case::zero_revision("revision", json!(0))]
#[case::zero_timeout("timeout_per_trial_ms", json!(0))]
#[case::invalid_eval("eval", json!("invalid name"))]
#[case::empty_scorers("scorers", json!([]))]
#[case::invalid_scorer("scorers", json!([{"kind":"called_before","first":"","then":"publish"}]))]
#[case::invalid_gate("gate", json!({"pass_rate":2.0}))]
#[tokio::test]
async fn should_validate_create_contract_bounds_before_storage(
    guarded_app: axum::Router,
    #[case] field: &str,
    #[case] value: Value,
) {
    let mut payload = create_payload();
    payload[field] = value;
    let response = guarded_app
        .oneshot(request("POST", "/lens/evals/runs", "team-a", Some(payload)))
        .await
        .unwrap();
    assert_eq!(response.status(), 422);
    assert_eq!(body::<Value>(response).await["code"], "invalid_request");
}

struct CountScorer(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl lens_server::eval_closer::ScoreRun for CountScorer {
    async fn score(
        &self,
        input: &lens_server::eval_closer::RunScoreInput,
    ) -> Result<lens_server::eval_closer::ScoredRun, lens_server::EvalCloserError> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        ErrorScorer.score(input).await
    }
}

#[rstest]
#[tokio::test]
async fn should_allow_only_one_closer_to_score_a_run(#[future(awt)] eval_fixture: EvalFixture) {
    let run = create(&eval_fixture).await;
    let stored = eval_fixture
        .store
        .finish("team-a", &run.id, Utc::now())
        .await
        .unwrap();
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let closer = lens_server::eval_closer::EvalCloser::new(
        eval_fixture.store.clone(),
        NoTraces,
        CountScorer(calls.clone()),
    );
    let now = Utc::now();
    let (first, second) = tokio::join!(closer.advance(&stored, now), closer.advance(&stored, now));
    first.unwrap();
    second.unwrap();
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    let done = eval_fixture.store.get("team-a", &run.id).await.unwrap();
    assert_eq!(done.run.status, RunStatus::Done);
}

#[rstest]
#[tokio::test]
async fn should_reject_results_for_cases_outside_the_selected_subset(
    #[future(awt)] eval_fixture: EvalFixture,
) {
    let mut payload = create_payload();
    payload["case_ids"] = json!(["case-2"]);
    let response = eval_fixture
        .app
        .clone()
        .oneshot(request("POST", "/lens/evals/runs", "team-a", Some(payload)))
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let run: EvalRun = body(response).await;
    assert_eq!(run.expected_trials, 3);
    let response = eval_fixture
        .app
        .clone()
        .oneshot(request(
            "PUT",
            &format!("/lens/evals/runs/{}/results/case-1/0", run.id),
            "team-a",
            Some(json!({"error":{"type":"Error","message":"failed"}})),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    assert_eq!(
        body::<ApiError>(response).await.code,
        ApiErrorCode::UnknownCase
    );
}
