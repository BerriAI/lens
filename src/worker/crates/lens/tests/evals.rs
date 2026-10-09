#[allow(
    dead_code,
    reason = "The shared HTTP fixture also supports activity-specific cases"
)]
mod activity {
    pub mod support;
}
mod evals {
    pub mod support;
}

use activity::support::{ADMIN, Database, database};
use evals::support::*;
use lens_contract::eval::RunStatus;
use lens_evals::RunRepository;
use litellm_storage_clickhouse::evals::Evals;
use rstest::rstest;
use serde_json::{Value, json};

#[rstest]
#[case::root_completed(("build-one","ok","",1),1,0)]
#[case::root_failed(("build-one","error","",1),0,0)]
#[case::otlp_failed(("build-one","STATUS_CODE_ERROR","",1),0,0)]
#[case::missing_version(("","ok","",1),0,1)]
#[case::version_mismatch(("wrong-build","ok","",1),0,1)]
#[case::idle_completion(("build-one","ok","external",130),1,0)]
#[tokio::test]
async fn real_trace_evaluation_finishes_durably_through_the_public_lifecycle(
    #[future(awt)] database: Database,
    #[case] trace_config: (&str, &str, &str, i64),
    #[case] passed: u64,
    #[case] errors: u64,
) {
    let (version, status, parent, age) = trace_config;
    trace(&database, "real-trace", version, status, parent, age).await;
    let server = serve(&database).await;
    let run = create(&server, "run", spec()).await;
    let repeated = create(&server, "run", spec()).await;
    assert_eq!(repeated.id, run.id);
    submit(
        &server,
        &run.id,
        json!({"trace":{"attribute":"session.id","value":"real-trace"},"cost_usd":0.25}),
    )
    .await;
    finish(&server, &run.id).await;
    evaluator(&database).tick().await.unwrap();
    let completed = get(&server, &run.id).await;
    assert_eq!(completed.status, RunStatus::Done, "{}", completed.failure);
    let summary = completed.summary.unwrap();
    assert_eq!(summary.passed, passed);
    assert_eq!(summary.errors, errors);
    assert_eq!(summary.cost_per_case, 0.25);
    assert_eq!(summary.total, 1);
    assert_eq!(summary.baseline_run_id, None);
    assert!(summary.gate.passed);
    let details: Value = server
        .request("GET", &format!("/lens/evals/runs/{}/details", run.id))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(details["run"]["status"], "done");
    assert_eq!(details["cases"][0]["input"], "Complete the task");
    assert_eq!(details["cases"][0]["verdict"], passed == 1);
    if errors == 0 {
        assert_eq!(details["cases"][0]["traces"][0]["trace_id"], "real-trace");
        assert!(
            !details["cases"][0]["traces"][0]["trace_ref"]
                .as_str()
                .unwrap()
                .is_empty()
        );
    }
    drop(server);
    let restarted = serve(&database).await;
    assert_eq!(get(&restarted, &run.id).await.summary.unwrap(), summary);
    let closed = restarted
        .request(
            "PUT",
            &format!("/lens/evals/runs/{}/results/case-one/0", run.id),
        )
        .json(&json!({"error":{"type":"Error","message":"late"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(closed.status(), 409);
    assert_eq!(
        closed.json::<Value>().await.unwrap(),
        json!({"detail":"run_closed","code":"run_closed"})
    );
}

#[rstest]
#[tokio::test]
async fn absent_traces_wait_and_then_use_the_remaining_trial_timeout(
    #[future(awt)] database: Database,
) {
    let server = serve(&database).await;
    let run = create(&server, "run", spec()).await;
    submit(
        &server,
        &run.id,
        json!({"trace":{"attribute":"trace_id","value":"late"}}),
    )
    .await;
    finish(&server, &run.id).await;
    evaluator(&database).tick().await.unwrap();
    assert_eq!(get(&server, &run.id).await.status, RunStatus::Scoring);
    trace(&database, "late", "build-one", "ok", "", 1).await;
    evaluator(&database).tick().await.unwrap();
    assert_eq!(get(&server, &run.id).await.summary.unwrap().passed, 1);
    let timed = create(&server, "timed", spec()).await;
    submit(
        &server,
        &timed.id,
        json!({"trace":{"value":"absent"},"duration_ms":60000}),
    )
    .await;
    finish(&server, &timed.id).await;
    evaluator(&database).tick().await.unwrap();
    let summary = get(&server, &timed.id).await.summary.unwrap();
    assert_eq!(summary.errors, 1);
    assert_eq!(summary.passed, 0);
}

#[rstest]
#[tokio::test]
async fn baseline_comparison_uses_real_main_verdicts_and_survives_restart(
    #[future(awt)] database: Database,
) {
    let server = serve(&database).await;
    trace(&database, "baseline", "build-one", "ok", "", 1).await;
    let baseline = create(&server, "baseline", spec()).await;
    submit(
        &server,
        &baseline.id,
        json!({"trace":{"attribute":"trace_id","value":"baseline"},"cost_usd":0.1}),
    )
    .await;
    finish(&server, &baseline.id).await;
    evaluator(&database).tick().await.unwrap();
    let mut candidate_spec = spec();
    candidate_spec["branch"] = json!("feature");
    candidate_spec["version"] = json!("build-two");
    let candidate = create(&server, "candidate", candidate_spec).await;
    submit(
        &server,
        &candidate.id,
        json!({"error":{"type":"RuntimeError","message":"agent failed"},"cost_usd":0.2}),
    )
    .await;
    finish(&server, &candidate.id).await;
    drop(server);
    evaluator(&database).tick().await.unwrap();
    let server = serve(&database).await;
    let summary = get(&server, &candidate.id).await.summary.unwrap();
    assert_eq!(
        summary.baseline_run_id.as_deref(),
        Some(baseline.id.as_str())
    );
    assert_eq!(summary.regressions.len(), 1);
    assert_eq!(summary.regressions[0].case_id, "case-one");
    assert!(!summary.regressions[0].critical);
    assert!(!summary.gate.passed);
    assert_eq!(summary.errors, 1);
}

#[rstest]
#[case::negative_trials("trials",json!(0))]
#[case::too_many_trials("trials",json!(11))]
#[case::empty_scorers("scorers",json!([]))]
#[case::empty_agent("agent",json!(""))]
#[case::invalid_name("eval",json!("Bad eval"))]
#[case::unknown_score("gate",json!({"min":{"missing":0.5}}))]
#[case::unbounded_timeout("timeout_per_trial_ms",json!(u64::MAX))]
#[tokio::test]
async fn invalid_specs_never_create_durable_runs(
    #[future(awt)] database: Database,
    #[case] field: &str,
    #[case] value: Value,
) {
    let server = serve(&database).await;
    let mut body = spec();
    body[field] = value;
    let response = server
        .request("POST", "/lens/evals/runs")
        .header("idempotency-key", "invalid")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 422);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"detail":"invalid_request","code":"invalid_request"})
    );
    assert!(
        Evals(database.sessions.clone())
            .list()
            .await
            .unwrap()
            .is_empty()
    );
}

#[rstest]
#[case::missing(None)]
#[case::unknown(Some("2"))]
#[tokio::test]
async fn unknown_contract_versions_fail_before_storage(
    #[future(awt)] database: Database,
    #[case] version: Option<&str>,
) {
    let server = serve(&database).await;
    let request = server
        .client
        .get(server.url.join("/lens/evals/runs").unwrap())
        .bearer_auth(ADMIN);
    let request = match version {
        Some(value) => request.header("x-lens-contract", value),
        None => request,
    };
    let response = request.send().await.unwrap();
    assert_eq!(response.status(), 409);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"detail":"contract_version","code":"contract_version"})
    );
}

#[rstest]
#[case::pass(0.8, 1)]
#[case::fail(0.2, 0)]
#[tokio::test]
async fn concurrent_schedulers_make_one_real_judge_request_with_trace_content(
    #[future(awt)] database: Database,
    #[case] score: f64,
    #[case] passed: u64,
) {
    use lens_analysis::{AnalysisModels, Catalog, Deployment, Provider, Secret};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    let provider = MockServer::start().await;
    Mock::given(method("POST")).and(path("/chat/completions")).respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_millis(100)).set_body_json(json!({
        "model":"fixture","choices":[{"message":{"content":json!({"score":score}).to_string()},"finish_reason":"stop"}],"usage":{"prompt_tokens":100,"completion_tokens":10}
    }))).expect(1).mount(&provider).await;
    let catalog=Catalog::parse(serde_json::to_vec(&json!({"fixture":{"mode":"chat","litellm_provider":"openai","input_cost_per_token":0.000001,"output_cost_per_token":0.000002,"max_input_tokens":10000,"max_output_tokens":400}})).unwrap().as_slice(),Default::default()).unwrap();
    let models = std::sync::Arc::new(
        AnalysisModels::new(
            std::sync::Arc::new(catalog),
            vec![Deployment {
                name: "judge".into(),
                model: "fixture".into(),
                provider: Provider::OpenAiCompatible,
                api_base: Some(provider.uri().parse().unwrap()),
                api_key: Secret::new("test-judge-key"),
                input_cost_per_token: None,
                output_cost_per_token: None,
                capacity: Default::default(),
                output_limits: Default::default(),
            }],
            Default::default(),
        )
        .unwrap(),
    );
    let server = serve(&database).await;
    trace(&database, "judged", "build-one", "ok", "", 1).await;
    let mut body = spec();
    body["scorers"] = json!([{"kind":"judge","prompt":"Did the agent complete the task?"}]);
    let run = create(&server, "judge", body).await;
    submit(
        &server,
        &run.id,
        json!({"trace":{"attribute":"session.id","value":"judged"},"cost_usd":0.1}),
    )
    .await;
    finish(&server, &run.id).await;
    let first = litellm_lens::evaluations::Evaluations::new(
        Evals(database.sessions.clone()),
        database.state.clone(),
        models.clone(),
    );
    let second = litellm_lens::evaluations::Evaluations::new(
        Evals(database.sessions.clone()),
        database.state.clone(),
        models,
    );
    let (left, right) = tokio::join!(first.tick(), second.tick());
    left.unwrap();
    right.unwrap();
    let completed = get(&server, &run.id).await;
    assert_eq!(completed.status, RunStatus::Done, "{}", completed.failure);
    assert_eq!(completed.summary.unwrap().passed, passed);
    let requests = provider.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let sent: String = String::from_utf8(requests[0].body.clone()).unwrap();
    assert!(sent.contains("Task completed with evidence"));
    assert!(sent.contains("Complete the task"));
    first.tick().await.unwrap();
    assert_eq!(provider.received_requests().await.unwrap().len(), 1);
}

#[rstest]
#[tokio::test]
async fn temporary_trace_storage_outage_releases_the_lease_and_retries(
    #[future(awt)] database: Database,
) {
    let server = serve(&database).await;
    trace(&database, "recoverable", "build-one", "ok", "", 1).await;
    let run = create(&server, "retry", spec()).await;
    submit(
        &server,
        &run.id,
        json!({"trace":{"attribute":"trace_id","value":"recoverable"},"cost_usd":0.1}),
    )
    .await;
    finish(&server, &run.id).await;
    database
        .state
        .schema_ready
        .store(false, std::sync::atomic::Ordering::Release);
    evaluator(&database).tick().await.unwrap();
    let pending = Evals(database.sessions.clone())
        .get(&run.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.run.status, RunStatus::Scoring);
    assert!(pending.lease.is_none());
    assert!(pending.run.failure.is_empty());
    database
        .state
        .schema_ready
        .store(true, std::sync::atomic::Ordering::Release);
    evaluator(&database).tick().await.unwrap();
    assert_eq!(get(&server, &run.id).await.summary.unwrap().passed, 1);
}

#[rstest]
#[case::viewer_read(
    "GET",
    "/lens/evals/runs",
    lens_contract::auth::Role::ProxyAdminViewer,
    200
)]
#[case::viewer_details(
    "GET",
    "/lens/evals/runs/details",
    lens_contract::auth::Role::ProxyAdminViewer,
    200
)]
#[case::viewer_write(
    "POST",
    "/lens/evals/runs",
    lens_contract::auth::Role::ProxyAdminViewer,
    403
)]
#[case::team_read(
    "GET",
    "/lens/evals/runs",
    lens_contract::auth::Role::InternalUser,
    403
)]
#[case::team_details(
    "GET",
    "/lens/evals/runs/details",
    lens_contract::auth::Role::InternalUser,
    403
)]
#[case::team_write(
    "POST",
    "/lens/evals/runs",
    lens_contract::auth::Role::InternalUser,
    403
)]
#[tokio::test]
async fn eval_http_permissions_preserve_the_dataset_access_boundary(
    #[future(awt)] database: Database,
    #[case] method: &str,
    #[case] path: &str,
    #[case] role: lens_contract::auth::Role,
    #[case] status: u16,
) {
    let server = serve(&database).await;
    let token = activity::support::delegated(lens_contract::auth::Identity {
        user_role: role,
        user_id: Some("caller".into()),
        team_id: Some("alpha".into()),
        ..Default::default()
    });
    let response = server
        .client
        .request(method.parse().unwrap(), server.url.join(path).unwrap())
        .bearer_auth(token)
        .header("x-lens-contract", "1")
        .json(&spec())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), status);
}

#[rstest]
#[case::dataset("dataset_id", json!("unknown"), 404, "dataset_not_found")]
#[case::revision("revision", json!(2), 404, "revision_not_found")]
#[case::case("case_ids", json!(["unknown"]), 422, "unknown_case")]
#[tokio::test]
async fn creation_rejects_unavailable_frozen_dataset_inputs(
    #[future(awt)] database: Database,
    #[case] field: &str,
    #[case] value: Value,
    #[case] status: u16,
    #[case] code: &str,
) {
    let server = serve(&database).await;
    let mut body = spec();
    body[field] = value;
    let response = server
        .request("POST", "/lens/evals/runs")
        .header("idempotency-key", "invalid")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), status);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"code":code,"detail":code})
    );
    assert!(
        Evals(database.sessions.clone())
            .list()
            .await
            .unwrap()
            .is_empty()
    );
}

#[rstest]
#[case::unknown_case("unknown", "0", json!({"trace":{"value":"trace"}}), "unknown_case")]
#[case::unknown_trial("case-one", "1", json!({"trace":{"value":"trace"}}), "invalid_result")]
#[case::invalid_trial("case-one", "-1", json!({"trace":{"value":"trace"}}), "invalid_result")]
#[case::empty_result("case-one", "0", json!({}), "invalid_result")]
#[case::both_results("case-one", "0", json!({"trace":{"value":"trace"},"error":{"type":"Error","message":"failed"}}), "invalid_result")]
#[tokio::test]
async fn invalid_submissions_leave_received_trials_unchanged(
    #[future(awt)] database: Database,
    #[case] case: &str,
    #[case] trial: &str,
    #[case] body: Value,
    #[case] code: &str,
) {
    let server = serve(&database).await;
    let run = create(&server, "run", spec()).await;
    let response = server
        .request(
            "PUT",
            &format!("/lens/evals/runs/{}/results/{case}/{trial}", run.id),
        )
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 422);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"code":code,"detail":code})
    );
    assert_eq!(get(&server, &run.id).await.received_trials, 0);
}

#[rstest]
#[tokio::test]
async fn repeated_create_keys_cannot_change_the_frozen_spec(#[future(awt)] database: Database) {
    let server = serve(&database).await;
    let original = create(&server, "same", spec()).await;
    let mut body = spec();
    body["version"] = json!("changed");
    let response = server
        .request("POST", "/lens/evals/runs")
        .header("idempotency-key", "same")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 409);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"code":"idempotency_key","detail":"idempotency_key"})
    );
    assert_eq!(get(&server, &original.id).await.version, "build-one");
}

#[rstest]
#[case::matched_gateway_spend(None, 0.37)]
#[case::explicit_sdk_cost(Some(0.11), 0.11)]
#[tokio::test]
async fn evaluation_costs_use_real_scoped_spend_unless_the_sdk_supplies_a_cost(
    #[future(awt)] database: Database,
    #[case] cost: Option<f64>,
    #[case] expected: f64,
) {
    let server = serve(&database).await;
    priced_trace(&database, "priced").await;
    let run = create(&server, "priced", spec()).await;
    submit(
        &server,
        &run.id,
        json!({"trace":{"attribute":"trace_id","value":"priced"},"cost_usd":cost}),
    )
    .await;
    finish(&server, &run.id).await;
    evaluator(&database).tick().await.unwrap();
    let completed = get(&server, &run.id).await;
    assert_eq!(completed.status, RunStatus::Done, "{}", completed.failure);
    assert_eq!(completed.summary.unwrap().cost_per_case, expected);
}

#[rstest]
#[tokio::test]
async fn missing_trials_affect_majority_error_counts_and_the_declared_trial_total(
    #[future(awt)] database: Database,
) {
    let server = serve(&database).await;
    trace(&database, "one-success", "build-one", "ok", "", 1).await;
    let mut body = spec();
    body["trials"] = json!(3);
    let run = create(&server, "majority", body).await;
    assert_eq!(run.expected_trials, 3);
    submit(
        &server,
        &run.id,
        json!({"trace":{"attribute":"trace_id","value":"one-success"},"cost_usd":0.12}),
    )
    .await;
    finish(&server, &run.id).await;
    evaluator(&database).tick().await.unwrap();
    let summary = get(&server, &run.id).await.summary.unwrap();
    assert_eq!(summary.total, 1);
    assert_eq!(summary.passed, 0);
    assert_eq!(summary.errors, 2);
    assert_eq!(summary.scores["task_completed"], 1.0 / 3.0);
}

#[rstest]
#[tokio::test]
async fn a_judge_run_with_no_configured_model_finishes_as_an_explicit_failure(
    #[future(awt)] database: Database,
) {
    let server = serve(&database).await;
    trace(&database, "judge-failure", "build-one", "ok", "", 1).await;
    let mut body = spec();
    body["scorers"] = json!([{"kind":"judge","prompt":"Was the task completed?"}]);
    let run = create(&server, "no-judge", body).await;
    submit(
        &server,
        &run.id,
        json!({"trace":{"attribute":"trace_id","value":"judge-failure"},"cost_usd":0.1}),
    )
    .await;
    finish(&server, &run.id).await;
    evaluator(&database).tick().await.unwrap();
    let failed = get(&server, &run.id).await;
    assert_eq!(failed.status, RunStatus::Failed);
    assert!(failed.summary.is_none());
    assert!(!failed.failure.is_empty());
    assert!(
        Evals(database.sessions.clone())
            .get(&run.id)
            .await
            .unwrap()
            .unwrap()
            .lease
            .is_none()
    );
}
