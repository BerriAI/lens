#[path = "evals/support.rs"]
mod support;

use axum::http::HeaderValue;
use chrono::Utc;
use lens_contract::eval::{ApiError, ApiErrorCode, EvalDefinition, EvalRun, RunStatus};
use rstest::rstest;
use serde_json::{Value, json};
use support::{
    EvalFixture, body, create, create_payload, eval_fixture, guarded_app, request,
    traced_eval_fixture,
};
use tower::ServiceExt;

fn output_agent_io() -> Value {
    json!({
        "version": 1,
        "connection": "agent",
        "submit": {"method": "POST", "path": "/complete", "accepted_status": 200, "json": {"input": ""}},
        "input": [{"source": "case.input", "target": "/input"}],
        "completion": {"kind": "immediate"},
        "output": {"pointer": "/output", "require_nonempty": true}
    })
}

#[rstest]
#[tokio::test]
async fn more_than_ten_trials_can_be_created_and_submitted(
    #[future(awt)] eval_fixture: EvalFixture,
) {
    let mut payload = create_payload();
    payload["trials"] = json!(11);
    let response = eval_fixture
        .app
        .clone()
        .oneshot(request("POST", "/lens/evals/runs", "team-a", Some(payload)))
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let run: EvalRun = body(response).await;
    assert_eq!(run.expected_trials, 22);
    let response = eval_fixture
        .app
        .clone()
        .oneshot(request(
            "PUT",
            &format!("/lens/evals/runs/{}/results/case-1/10", run.id),
            "team-a",
            Some(json!({"trace": {"value": "session-11"}})),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 204);
    let stored = eval_fixture.store.get("team-a", &run.id).await.unwrap();
    assert_eq!(stored.trials[0].trial, 10);
    assert_eq!(
        stored.trials[0].result.trace.as_ref().unwrap().value,
        "session-11"
    );
}

#[rstest]
#[tokio::test]
async fn long_error_results_are_stored_without_truncation(
    #[future(awt)] eval_fixture: EvalFixture,
) {
    let run = create(&eval_fixture).await;
    let message = "x".repeat(2 * 1024 * 1024 + 1);
    let write = request(
        "PUT",
        &format!("/lens/evals/runs/{}/results/case-1/0", run.id),
        "team-a",
        Some(json!({"error": {"type": "Error", "message": message}})),
    );
    let response = eval_fixture.app.clone().oneshot(write).await.unwrap();
    assert_eq!(response.status(), 204);
    let stored = eval_fixture.store.get("team-a", &run.id).await.unwrap();
    assert_eq!(
        stored.trials[0].result.error.as_ref().unwrap().message,
        message
    );
}

fn v2(mut request: axum::http::Request<axum::body::Body>) -> axum::http::Request<axum::body::Body> {
    request
        .headers_mut()
        .insert("x-lens-contract", HeaderValue::from_static("2"));
    request
}

#[rstest]
#[tokio::test]
async fn named_eval_contract_is_persisted_without_breaking_legacy_readers(
    #[future(awt)] eval_fixture: EvalFixture,
) {
    let mut payload = spec(1);
    payload["agent_io"] = output_agent_io();
    payload["scorers"] =
        json!([{"kind": "judge", "prompt": "Check the expected answer", "model": ""}]);
    let response = eval_fixture
        .app
        .clone()
        .oneshot(v2(request(
            "PUT",
            "/lens/evals/named",
            "team-a",
            Some(payload.clone()),
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let definition: EvalDefinition = body(response).await;
    assert!(definition.spec.agent_io.is_some());

    for (version, has_agent_io) in [("1", false), ("2", true)] {
        for path in ["/lens/evals/named", "/lens/evals"] {
            let mut read = request("GET", path, "team-a", None);
            read.headers_mut()
                .insert("x-lens-contract", HeaderValue::from_str(version).unwrap());
            let response = eval_fixture.app.clone().oneshot(read).await.unwrap();
            assert_eq!(response.status(), 200);
            let result: Value = body(response).await;
            let definition = if path == "/lens/evals" {
                &result[0]
            } else {
                &result
            };
            assert_eq!(definition["spec"].get("agent_io").is_some(), has_agent_io);
        }
    }
    payload.as_object_mut().unwrap().remove("agent_io");
    let overwrite = eval_fixture
        .app
        .clone()
        .oneshot(request("PUT", "/lens/evals/named", "team-a", Some(payload)))
        .await
        .unwrap();
    assert_eq!(overwrite.status(), 422);
    assert_eq!(
        eval_fixture
            .store
            .definition("team-a", "named")
            .await
            .unwrap(),
        definition
    );
}

#[rstest]
#[case::v1("1", 422)]
#[case::v2("2", 204)]
#[tokio::test]
async fn output_results_require_v2_and_remain_inspectable(
    #[future(awt)] eval_fixture: EvalFixture,
    #[case] version: &str,
    #[case] status: u16,
) {
    let run = create(&eval_fixture).await;
    let mut write = request(
        "PUT",
        &format!("/lens/evals/runs/{}/results/case-1/0", run.id),
        "team-a",
        Some(json!({"output": "the completed answer"})),
    );
    write
        .headers_mut()
        .insert("x-lens-contract", HeaderValue::from_str(version).unwrap());
    let response = eval_fixture.app.clone().oneshot(write).await.unwrap();
    assert_eq!(response.status(), status);
    let stored = eval_fixture.store.get("team-a", &run.id).await.unwrap();
    assert_eq!(stored.cases[0].input, "first");
    assert_eq!(stored.trials.len(), usize::from(status == 204));
    if status == 204 {
        assert_eq!(
            stored.trials[0].result.output.as_deref(),
            Some("the completed answer")
        );
        for (version, expected) in [("1", None), ("2", Some("the completed answer"))] {
            let mut read = request(
                "GET",
                &format!("/lens/evals/runs/{}/cases/case-1", run.id),
                "team-a",
                None,
            );
            read.headers_mut()
                .insert("x-lens-contract", HeaderValue::from_str(version).unwrap());
            let response = eval_fixture.app.clone().oneshot(read).await.unwrap();
            assert_eq!(response.status(), 200);
            assert_eq!(
                body::<Value>(response).await["trials"][0]
                    .get("output")
                    .and_then(Value::as_str),
                expected
            );
        }
    }
}

#[rstest]
#[case::v1("1", false)]
#[case::v2_trace_scorer("2", false)]
#[case::invalid_mapping("2", true)]
#[tokio::test]
async fn invalid_named_eval_contracts_fail_before_storage(
    guarded_app: axum::Router,
    #[case] version: &str,
    #[case] invalid_mapping: bool,
) {
    let mut payload = spec(1);
    payload["agent_io"] = output_agent_io();
    if invalid_mapping {
        payload["agent_io"]["input"][0]["target"] = "/missing".into();
        payload["scorers"] = json!([{"kind":"judge", "prompt":"Check answer", "model":""}]);
    }
    let mut write = request("PUT", "/lens/evals/named", "team-a", Some(payload));
    write
        .headers_mut()
        .insert("x-lens-contract", HeaderValue::from_str(version).unwrap());
    let response = guarded_app.oneshot(write).await.unwrap();
    assert_eq!(response.status(), 422);
}

#[rstest]
#[tokio::test]
async fn output_only_contract_cannot_claim_unmeasured_cost_savings(guarded_app: axum::Router) {
    let mut payload = spec(1);
    payload["agent_io"] = output_agent_io();
    payload["scorers"] = json!([{"kind":"judge", "prompt":"Check answer", "model":""}]);
    payload["gate"] = json!({"cost_per_case": 0.1});
    let response = guarded_app
        .oneshot(v2(request(
            "PUT",
            "/lens/evals/named",
            "team-a",
            Some(payload),
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), 422);
}

#[rstest]
#[case::legacy("", false)]
#[case::disabled("&include_ci=false", false)]
#[case::dashboard("&include_ci=true", true)]
#[tokio::test]
async fn ci_provenance_is_only_returned_when_requested_by_the_dashboard(
    #[future(awt)] eval_fixture: EvalFixture,
    #[case] query: &str,
    #[case] includes_ci: bool,
) {
    let ci_url = "https://github.com/example/agent/actions/runs/42";
    let mut payload = create_payload();
    payload["ci_url"] = ci_url.into();
    let created = eval_fixture
        .app
        .clone()
        .oneshot(request("POST", "/lens/evals/runs", "team-a", Some(payload)))
        .await
        .unwrap();
    assert_eq!(created.status(), 201);
    let created: Value = body(created).await;
    assert!(created.get("ci_url").is_none());
    let created: EvalRun = serde_json::from_value(created).unwrap();
    let read = eval_fixture
        .app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/lens/evals/runs/{}", created.id),
            "team-a",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(read.status(), 200);
    assert!(body::<Value>(read).await.get("ci_url").is_none());
    let list = eval_fixture
        .app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/lens/evals/runs?agent=agent{query}"),
            "team-a",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(list.status(), 200);
    let list: Vec<Value> = body(list).await;
    assert_eq!(list.len(), 1);
    assert_eq!(
        list[0].get("ci_url").and_then(Value::as_str),
        includes_ci.then_some(ci_url)
    );
    let finished = eval_fixture
        .app
        .clone()
        .oneshot(request(
            "POST",
            &format!("/lens/evals/runs/{}/finish", created.id),
            "team-a",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(finished.status(), 202);
    assert!(body::<Value>(finished).await.get("ci_url").is_none());
    assert_eq!(
        eval_fixture
            .store
            .get("team-a", &created.id)
            .await
            .unwrap()
            .request
            .ci_url,
        ci_url
    );
}

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
    assert_eq!(stored.cases[0].input, "first");
    assert_eq!(
        stored.cases[0].followups,
        vec!["Translate the answer to Spanish", "Include the test names"]
    );
    assert!(stored.cases[1].followups.is_empty());
    let definitions = eval_fixture
        .app
        .clone()
        .oneshot(request("GET", "/lens/evals", "team-a", None))
        .await
        .unwrap();
    assert_eq!(definitions.status(), 200);
    let definitions: Vec<EvalDefinition> = body(definitions).await;
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].name, first.eval);
    let definition = eval_fixture
        .app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/lens/evals/{}", first.eval),
            "team-a",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(definition.status(), 200);
    assert_eq!(body::<EvalDefinition>(definition).await, definitions[0]);
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
#[case::case_list("GET", "/cases", None)]
#[case::case_detail("GET", "/cases/case-1", None)]
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
    #[values(None, Some("3"), Some("1.5"))] version: Option<&str>,
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
        json!({"finding_id":"finding-1","repo_url":"https://example.test/repo"})
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

#[rstest]
#[case::selected(true, 201)]
#[case::missing(false, 422)]
#[tokio::test]
async fn explicit_baseline_is_bound_or_rejected_without_implicit_fallback(
    #[future(awt)] eval_fixture: EvalFixture,
    #[case] selected: bool,
    #[case] status: u16,
) {
    let baseline = create(&eval_fixture).await;
    let now = Utc::now();
    let finished = eval_fixture
        .store
        .finish("team-a", &baseline.id, now)
        .await
        .unwrap();
    lens_server::eval_closer::EvalCloser::new(eval_fixture.store.clone(), NoTraces, ErrorScorer)
        .advance(&finished, now)
        .await
        .unwrap();
    let mut payload = create_payload();
    payload["branch"] = json!("candidate");
    payload["baseline_run_id"] = json!(if selected {
        baseline.id.as_str()
    } else {
        "missing"
    });
    let response = eval_fixture
        .app
        .clone()
        .oneshot(request("POST", "/lens/evals/runs", "team-a", Some(payload)))
        .await
        .unwrap();
    assert_eq!(response.status(), status);
    if selected {
        let candidate: EvalRun = body(response).await;
        let stored = eval_fixture
            .store
            .get("team-a", &candidate.id)
            .await
            .unwrap();
        assert_eq!(
            stored.request.baseline_run_id.as_deref(),
            Some(baseline.id.as_str())
        );
        assert_eq!(
            eval_fixture
                .store
                .baseline(&stored)
                .await
                .unwrap()
                .unwrap()
                .run
                .id,
            baseline.id
        );
    } else {
        assert_eq!(body::<Value>(response).await["code"], "invalid_request");
    }
}

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
    let response = eval_fixture
        .app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/lens/evals/runs/{}/cases", run.id),
            "team-a",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let cases: Vec<lens_contract::eval::RunCaseSummary> = body(response).await;
    assert_eq!(cases.len(), 2);
    assert_eq!(cases[0].passed, Some(false));
    assert_eq!(cases[1].passed, Some(false));
}

#[rstest]
#[case::zero_trials("trials", json!(0))]
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

struct TransientTraces {
    unavailable: String,
    failed: std::sync::atomic::AtomicBool,
    closed_at_ms: i64,
}

impl lens_server::eval_closer::TraceSource for TransientTraces {
    async fn trace(
        &self,
        _team: &str,
        reference: &lens_contract::eval::TraceRef,
    ) -> Result<Option<litellm_traces_clickhouse::evals::EvalTrace>, lens_server::EvalCloserError>
    {
        if reference.value == self.unavailable
            && !self.failed.swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            return Err(lens_server::EvalCloserError::TransientTraces(Box::new(
                std::io::Error::new(std::io::ErrorKind::TimedOut, "temporary read outage"),
            )));
        }
        Ok(Some(litellm_traces_clickhouse::evals::EvalTrace {
            traces: Vec::new(),
            spans: vec![litellm_traces_clickhouse::evals::EvalSpan {
                span_id: "root".into(),
                parent_span_id: String::new(),
                name: "completed".into(),
                start_ns: (self.closed_at_ms - 1) * 1_000_000,
                end_ns: self.closed_at_ms * 1_000_000,
                status: litellm_traces::SpanStatus::Ok,
                attributes: Default::default(),
                input: String::new(),
                output: String::new(),
            }],
            root_ended_at_ms: Some(self.closed_at_ms),
            last_received_at_ms: self.closed_at_ms,
            agent: "agent".into(),
            version: "abc123".into(),
            environment: "lens-eval".into(),
            gateway_cost_usd: 0.0,
        }))
    }
}

struct TraceScorer(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl lens_server::eval_closer::ScoreRun for TraceScorer {
    async fn score(
        &self,
        input: &lens_server::eval_closer::RunScoreInput,
    ) -> Result<lens_server::eval_closer::ScoredRun, lens_server::EvalCloserError> {
        self.0.fetch_add(
            input.trials.iter().map(|trial| trial.spans.len()).sum(),
            std::sync::atomic::Ordering::SeqCst,
        );
        ErrorScorer.score(input).await
    }
}

async fn prepare_trace_run(fixture: &EvalFixture) {
    let run = create(fixture).await;
    fixture
        .store
        .put_result(
            "team-a",
            &run.id,
            "case-1",
            0,
            lens_contract::eval::CaseResult {
                trace: Some(lens_contract::eval::TraceRef {
                    attribute: lens_contract::eval::TraceAttribute::TraceId,
                    value: run.id.clone(),
                }),
                ..Default::default()
            },
            Utc::now(),
        )
        .await
        .unwrap();
    fixture
        .store
        .finish("team-a", &run.id, Utc::now())
        .await
        .unwrap();
}

#[rstest]
#[tokio::test]
async fn should_retry_transient_trace_reads_without_blocking_other_runs(
    #[future(awt)] eval_fixture: EvalFixture,
) {
    let now = Utc::now();
    prepare_trace_run(&eval_fixture).await;
    prepare_trace_run(&eval_fixture).await;
    let queued = eval_fixture.store.scoring("", 100).await.unwrap();
    assert_eq!(queued.runs.len(), 2);
    let first = &queued.runs[0].run.id;
    let second = &queued.runs[1].run.id;
    let scored_spans = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let closer = lens_server::eval_closer::EvalCloser::new(
        eval_fixture.store.clone(),
        TransientTraces {
            unavailable: first.clone(),
            failed: std::sync::atomic::AtomicBool::new(false),
            closed_at_ms: now.timestamp_millis(),
        },
        TraceScorer(scored_spans.clone()),
    );
    assert!(matches!(
        closer.tick(now).await,
        Err(lens_server::EvalCloserError::TransientTraces(_))
    ));
    let pending = eval_fixture.store.get("team-a", first).await.unwrap();
    assert_eq!(pending.run.status, RunStatus::Scoring);
    assert!(pending.run.failure.is_empty());
    assert!(pending.scoring_lease.is_none());
    let completed = eval_fixture.store.get("team-a", second).await.unwrap();
    assert_eq!(completed.run.status, RunStatus::Done);
    assert_eq!(scored_spans.load(std::sync::atomic::Ordering::SeqCst), 1);
    closer.tick(Utc::now()).await.unwrap();
    let retried = eval_fixture.store.get("team-a", first).await.unwrap();
    assert_eq!(retried.run.status, RunStatus::Done);
    assert_eq!(retried.run.summary.unwrap().errors, 5);
    assert_eq!(scored_spans.load(std::sync::atomic::Ordering::SeqCst), 2);
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

fn spec(trials: u32) -> Value {
    json!({
        "agent": "moyai",
        "dataset_id": "dataset-1",
        "scorers": [{"kind": "task_completed"}, {"kind": "called_before", "first": "run_tests", "then": "open_pr"}],
        "trials": trials,
        "gate": {"regressions": 0, "pass_rate": 0.9},
    })
}

#[rstest]
#[tokio::test]
async fn should_store_eval_definitions_per_team_and_update_in_place(
    #[future(awt)] eval_fixture: EvalFixture,
) {
    let put = |team: &str, trials: u32| {
        request(
            "PUT",
            "/lens/evals/agent-regressions",
            team,
            Some(spec(trials)),
        )
    };
    let first = eval_fixture
        .app
        .clone()
        .oneshot(put("team-a", 3))
        .await
        .unwrap();
    assert_eq!(first.status(), 200);
    let first: EvalDefinition = body(first).await;
    let repeat: EvalDefinition = body(
        eval_fixture
            .app
            .clone()
            .oneshot(put("team-a", 3))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(repeat, first);
    let updated: EvalDefinition = body(
        eval_fixture
            .app
            .clone()
            .oneshot(put("team-a", 5))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(updated.spec.trials, 5);
    assert_eq!(updated.spec.baseline, "main");
    assert_eq!(updated.spec.revision, None);

    let read: EvalDefinition = body(
        eval_fixture
            .app
            .clone()
            .oneshot(request(
                "GET",
                "/lens/evals/agent-regressions",
                "team-a",
                None,
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(read, updated);
    let listed: Vec<EvalDefinition> = body(
        eval_fixture
            .app
            .clone()
            .oneshot(request("GET", "/lens/evals", "team-a", None))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(listed, vec![updated]);

    let other = eval_fixture
        .app
        .clone()
        .oneshot(request(
            "GET",
            "/lens/evals/agent-regressions",
            "team-b",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(other.status(), 404);
    let error: ApiError = body(other).await;
    assert_eq!(error.code, ApiErrorCode::EvalNotFound);
}

#[rstest]
#[case::reserved("runs", spec(1), 405)]
#[case::uppercase("Agent", spec(1), 422)]
#[case::no_scorers("agent", json!({"agent": "moyai", "dataset_id": "dataset-1", "scorers": []}), 422)]
#[case::zero_trials("agent", spec(0), 422)]
#[case::unknown_field("agent", json!({"agent": "moyai", "dataset_id": "d", "scorers": [{"kind": "task_completed"}], "extra": 1}), 422)]
#[tokio::test]
async fn should_reject_invalid_eval_definitions(
    #[future(awt)] eval_fixture: EvalFixture,
    #[case] name: &str,
    #[case] payload: Value,
    #[case] status: u16,
) {
    let response = eval_fixture
        .app
        .clone()
        .oneshot(request(
            "PUT",
            &format!("/lens/evals/{name}"),
            "team-a",
            Some(payload),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), status);
}

#[rstest]
#[case::matching("1")]
#[case::conflicting("2")]
#[tokio::test]
async fn duplicate_contract_headers_fail_before_storage(
    guarded_app: axum::Router,
    #[case] duplicate: &str,
) {
    let mut request = request("GET", "/lens/evals/runs", "team-a", None);
    request
        .headers_mut()
        .append("x-lens-contract", HeaderValue::from_str(duplicate).unwrap());
    let response = guarded_app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), 409);
    assert_eq!(
        body::<ApiError>(response).await.code,
        ApiErrorCode::ContractVersion
    );
}

#[rstest]
#[tokio::test]
async fn standalone_admin_can_use_evals_without_a_gateway_team(
    #[future(awt)] eval_fixture: EvalFixture,
) {
    let spec =
        json!({"agent":"standalone","dataset_id":"dataset","scorers":[{"kind":"task_completed"}]});
    let mut write = request("PUT", "/lens/evals/local", "team-a", Some(spec.clone()));
    write.headers_mut().insert(
        "authorization",
        HeaderValue::from_static("Bearer eval-route-test-admin-token-32-characters"),
    );
    let written = eval_fixture.app.clone().oneshot(write).await.unwrap();
    assert_eq!(written.status(), 200);
    assert_eq!(body::<Value>(written).await["spec"]["agent"], "standalone");
    let mut read = request("GET", "/lens/evals", "team-a", None);
    read.headers_mut().insert(
        "authorization",
        HeaderValue::from_static("Bearer eval-route-test-admin-token-32-characters"),
    );
    let listed = eval_fixture.app.clone().oneshot(read).await.unwrap();
    assert_eq!(listed.status(), 200);
    let definitions: Vec<EvalDefinition> = body(listed).await;
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].name, "local");
    let scoped = eval_fixture
        .app
        .clone()
        .oneshot(request("GET", "/lens/evals/local", "team-a", None))
        .await
        .unwrap();
    assert_eq!(scoped.status(), 404);
}

#[rstest]
#[case::unknown_gate("gate", json!({"min":{"missing":0.5}}))]
#[case::gate_above_one("gate", json!({"min":{"task_completed":1.1}}))]
#[case::unbounded_timeout("timeout_per_trial_ms", json!(u64::MAX))]
#[tokio::test]
async fn scoring_validation_prevents_unusable_durable_runs(
    #[future(awt)] eval_fixture: EvalFixture,
    #[case] field: &str,
    #[case] value: Value,
) {
    let mut payload = create_payload();
    payload[field] = value;
    let response = eval_fixture
        .app
        .clone()
        .oneshot(request("POST", "/lens/evals/runs", "team-a", Some(payload)))
        .await
        .unwrap();
    assert_eq!(response.status(), 422);
    assert_eq!(body::<Value>(response).await["code"], "invalid_request");
    assert!(
        eval_fixture
            .store
            .list("team-a", &Default::default())
            .await
            .unwrap()
            .is_empty()
    );
}

#[rstest]
#[tokio::test]
async fn should_list_every_pinned_run_case_without_a_verdict_change(
    #[future(awt)] eval_fixture: EvalFixture,
) {
    let run = create(&eval_fixture).await;
    let response = eval_fixture
        .app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/lens/evals/runs/{}/cases", run.id),
            "team-a",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let cases: Vec<lens_contract::eval::RunCaseSummary> = body(response).await;
    assert_eq!(cases.len(), 2);
    assert_eq!(cases[0].case_id, "case-1");
    assert_eq!(cases[0].title, "Run tests");
    assert!(cases[0].critical);
    assert_eq!(cases[0].passed, None);
    assert_eq!(cases[1].case_id, "case-2");
}

#[rstest]
#[case::trace("trace_id", "trace-a", vec!["trace-a"])]
#[case::session("session.id", "session-a", vec!["trace-a", "trace-b"])]
#[case::foreign_trace("trace_id", "private-trace", vec![])]
#[tokio::test]
async fn should_link_only_traces_resolved_for_the_runs_owner(
    #[future(awt)] traced_eval_fixture: EvalFixture,
    #[case] attribute: &str,
    #[case] value: &str,
    #[case] expected: Vec<&str>,
) {
    let run = create(&traced_eval_fixture).await;
    let response = traced_eval_fixture
        .app
        .clone()
        .oneshot(request(
            "PUT",
            &format!("/lens/evals/runs/{}/results/case-1/0", run.id),
            "team-a",
            Some(json!({"trace": {"attribute": attribute, "value": value}})),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 204);
    let response = traced_eval_fixture
        .app
        .clone()
        .oneshot(request(
            "GET",
            &format!("/lens/evals/runs/{}/cases/case-1", run.id),
            "team-a",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let case: lens_contract::eval::RunCase = body(response).await;
    let traces = &case.trials[0].traces;
    assert_eq!(
        traces
            .iter()
            .map(|trace| trace.trace_id.as_str())
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        traces
            .iter()
            .map(|trace| trace.trace_ref.len())
            .collect::<Vec<_>>(),
        vec![64; expected.len()]
    );
}
