mod datasets {
    pub mod support;
}

use datasets::support::{ADMIN, Database, database};
use lens_analysis::{Deployment, Provider, Secret};
use lens_contract::{
    ingestion::IngestionKeyCreated,
    investigations::{Lens, LensList},
    worker::JobStatus,
};
use lens_inference::{ModelCapacity, OutputLimits};
use rstest::{fixture, rstest};
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[fixture]
fn deployment(#[default("http://127.0.0.1:1/v1".into())] endpoint: String
) -> Deployment {
    Deployment {
        name: "test-analysis".into(),
        model: "gpt-4o-mini".into(),
        provider: Provider::OpenAiCompatible,
        api_base: Some(endpoint.parse().unwrap()),
        api_key: Secret::new("private-fixture-provider-key"),
        input_cost_per_token: Some(0.000001),
        output_cost_per_token: Some(0.000002),
        capacity: ModelCapacity {
            max_input_tokens: Some(1_000_000.try_into().unwrap()),
            max_output_tokens: Some(4096.try_into().unwrap()),
        },
        output_limits: OutputLimits {
            max_tokens: Some(1024.try_into().unwrap()),
            ..Default::default()
        },
    }
}

#[fixture]
fn trace() -> Value {
    let now = (chrono::Utc::now() - chrono::TimeDelta::minutes(5))
        .timestamp_nanos_opt()
        .unwrap();
    json!({"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"local-worker-test"}}]},"scopeSpans":[{"spans":[{
        "traceId":"aabbccdd00112233aabbccdd00112233","spanId":"aabbccdd00112233","name":"Test agent",
        "startTimeUnixNano":now.to_string(),"endTimeUnixNano":(now+1000000).to_string(),
        "attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"invoke_agent"}},{"key":"gen_ai.agent.name","value":{"stringValue":"local-worker-test"}},
            {"key":"gen_ai.input.messages","value":{"stringValue":"[{\"role\":\"user\",\"content\":\"Check my invoice\"}]"}},
            {"key":"gen_ai.output.messages","value":{"stringValue":"[{\"role\":\"assistant\",\"content\":\"Your invoice is paid\"}]"}}],"status":{"code":1}
    }]}]}]})
}

#[rstest]
#[case::complete(200, 100.0, JobStatus::Completed, 1)]
#[case::provider_failure(400, 100.0, JobStatus::Failed, 1)]
#[case::budget_denial(200, 0.00000001, JobStatus::Failed, 0)]
#[tokio::test]
async fn standalone_investigation_uses_local_storage_and_settles_budget(
    #[future(awt)] database: Database,
    trace: Value,
    #[case] provider_status: u16,
    #[case] budget: f64,
    #[case] expected: JobStatus,
    #[case] calls: usize,
) {
    let provider = MockServer::start().await;
    let completion = json!({"result":{"observations":[],"cannot_assess":false,"reasoning":"The trace reports the invoice status"}});
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(ResponseTemplate::new(provider_status).set_body_json(json!({
        "id":"completion-fixture","object":"chat.completion","created":1,"model":"gpt-4o-mini",
        "choices":[{"index":0,"message":{"role":"assistant","content":completion.to_string()},"finish_reason":"stop"}],
        "usage":{"prompt_tokens":10,"completion_tokens":20,"total_tokens":30}
    }))).mount(&provider).await;
    let server = database
        .serve_models(true, vec![deployment(format!("{}/v1", provider.uri()))])
        .await;
    let client = reqwest::Client::new();
    let readiness: LensList = client
        .get(format!("{}/lens", server.url))
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(readiness.tracing_enabled);
    assert_eq!(readiness.workers.len(), 1);
    assert!(readiness.workers[0].analysis_key_id.is_some());
    let models: Value = client
        .get(format!("{}/model_group/info", server.url))
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(models["data"][0]["model_group"], "test-analysis");
    assert_eq!(models["data"][0]["mode"], "chat");
    let key = client
        .post(format!("{}/lens/tracing/keys", server.url))
        .bearer_auth(ADMIN)
        .json(&json!({"name":"Test agent"}))
        .send()
        .await
        .unwrap();
    assert_eq!(key.status(), 200, "{}", key.text().await.unwrap());
    let key: IngestionKeyCreated = key.json().await.unwrap();
    let ingested = client
        .post(format!("{}/v1/traces", server.url))
        .bearer_auth(key.key)
        .json(&trace)
        .send()
        .await
        .unwrap();
    assert_eq!(ingested.status(), 200, "{}", ingested.text().await.unwrap());
    let created=client.post(format!("{}/lens",server.url)).bearer_auth(ADMIN).json(&json!({"name":"Local investigation","model":"test-analysis","agent_name":"local-worker-test","monthly_budget":budget,"checks":[{"id":"invoice","instruction":"The response must match the invoice status"}]})).send().await.unwrap();
    assert_eq!(created.status(), 200, "{}", created.text().await.unwrap());
    let created: Lens = created.json().await.unwrap();
    let cancelled = client
        .post(format!("{}/lens/{}/cancel", server.url, created.id))
        .bearer_auth(ADMIN)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        cancelled.status(),
        200,
        "{}",
        cancelled.text().await.unwrap()
    );
    let now = chrono::Utc::now();
    let queued=client.post(format!("{}/lens/{}/runs",server.url,created.id)).bearer_auth(ADMIN).json(&json!({"start":now-chrono::TimeDelta::hours(1),"end":now+chrono::TimeDelta::seconds(1)})).send().await.unwrap();
    assert_eq!(queued.status(), 200, "{}", queued.text().await.unwrap());
    let worker = server.worker.as_ref().unwrap();
    let (first, second) = tokio::join!(worker.run_once(), worker.run_once());
    assert_ne!(
        first.unwrap(),
        second.unwrap(),
        "only one concurrent worker should claim the job"
    );
    let read = client
        .get(format!("{}/lens/{}", server.url, created.id))
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(read.status(), 200);
    let completed: Lens = read.json().await.unwrap();
    assert_eq!(
        completed.jobs[0].status, expected,
        "{}",
        completed.jobs[0].error
    );
    assert_eq!(completed.jobs[0].attempts, 1);
    assert_eq!(
        completed.jobs[0].sample.as_ref().unwrap().executions.len(),
        1
    );
    assert!(completed.reservations.is_empty());
    let requests = provider.received_requests().await.unwrap();
    assert_eq!(requests.len(), calls);
    if expected == JobStatus::Completed {
        assert_eq!(completed.jobs[0].coverage.screened, 1);
        assert_eq!(completed.jobs[0].assessments.len(), 1);
        assert!(!completed.jobs[0].assessments[0].cannot_assess);
        assert!(completed.spent > 0.0);
        assert_eq!(completed.spent, completed.jobs[0].cost);
        assert_eq!(
            completed.jobs[0]
                .steps
                .iter()
                .filter(|step| step.kind == lens_contract::worker::StepKind::Model)
                .count(),
            1
        );
    } else {
        assert!(!completed.jobs[0].error.is_empty());
        assert_eq!(completed.spent, 0.0);
    }
    drop(server);
    let restarted = database
        .serve_models(true, vec![deployment(format!("{}/v1", provider.uri()))])
        .await;
    let saved = client
        .get(format!("{}/lens/{}", restarted.url, created.id))
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(saved.status(), 200);
    let saved: Lens = saved.json().await.unwrap();
    assert_eq!(
        serde_json::to_value(&saved).unwrap(),
        serde_json::to_value(&completed).unwrap()
    );
    assert!(!restarted.worker.as_ref().unwrap().run_once().await.unwrap());
}

#[rstest]
#[case::due_schedule(true, -60, false, "test-analysis", "test-analysis", true)]
#[case::future_schedule(true, 3600, false, "test-analysis", "test-analysis", false)]
#[case::paused_schedule(false, -60, false, "test-analysis", "test-analysis", false)]
#[case::removed_schedule_model(true, -60, false, "removed-model", "test-analysis", false)]
#[case::queued_while_paused(false, 3600, true, "test-analysis", "test-analysis", true)]
#[case::queued_supported_model(true, 3600, true, "removed-model", "test-analysis", true)]
#[case::queued_removed_model(true, -60, true, "test-analysis", "removed-model", false)]
#[tokio::test]
async fn claims_respect_schedule_and_frozen_job_model(
    #[future(awt)] database: Database,
    #[case] enabled: bool,
    #[case] due_offset: i64,
    #[case] queued: bool,
    #[case] configured_model: &str,
    #[case] queued_model: &str,
    #[case] expected_claim: bool,
) {
    use lens_contract::{
        investigations::Scope,
        worker::{JobTrigger, LensSettings},
    };
    use lens_investigations::{LensRepository, create_lens};
    let server = database
        .serve_models(true, vec![deployment("http://127.0.0.1:1/v1".into())])
        .await;
    let worker = server.worker.as_ref().unwrap();
    let now = chrono::Utc::now();
    let settings: LensSettings = serde_json::from_value(json!({"name":"Scheduled model eligibility","model":queued_model,"checks":[{"id":"correct","instruction":"The response must match the evidence"}]})).unwrap();
    let initial = create_lens(
        settings,
        Scope {
            all_teams: true,
            ..Default::default()
        },
        now - chrono::TimeDelta::minutes(1),
        "schedule-fixture",
        "queued-fixture",
    )
    .unwrap();
    let record = Lens {
        settings: LensSettings {
            enabled,
            model: configured_model.try_into().unwrap(),
            ..initial.settings.clone()
        },
        next_run_at: now + chrono::TimeDelta::seconds(due_offset),
        jobs: if queued { initial.jobs.clone() } else { vec![] },
        ..initial
    };
    let saved = worker.repository.create(&record).await.unwrap();
    let claimed = worker.claim().await.unwrap();
    assert_eq!(claimed.is_some(), expected_claim);
    let current = worker.repository.get(&record.id).await.unwrap().unwrap();
    if let Some(claim) = claimed {
        assert_eq!(claim.lens_id, record.id);
        assert_eq!(claim.job.status, JobStatus::Running);
        assert_eq!(claim.job.attempts, 1);
        assert_eq!(claim.job.settings.model.as_str(), "test-analysis");
        assert_eq!(claim.job.trigger, JobTrigger::Schedule);
        if queued {
            assert_eq!(claim.job.id, "queued-fixture");
        } else {
            assert_ne!(claim.job.id, "queued-fixture");
        }
        assert_eq!(current.jobs[0].id, claim.job.id);
        assert!(worker.claim().await.unwrap().is_none());
    } else {
        assert_eq!(
            serde_json::to_value(current).unwrap(),
            serde_json::to_value(saved).unwrap()
        );
    }
}

#[rstest]
#[case::missing_analysis(false, false, false)]
#[case::revoked(true, true, true)]
#[case::available(true, false, false)]
#[tokio::test]
async fn revoked_or_unconfigured_workers_do_not_claim(
    #[future(awt)] database: Database,
    #[case] configured: bool,
    #[case] revoked: bool,
    #[case] unauthorized: bool,
) {
    use lens_investigations::{LensRepository, WorkerRepository, create_lens};
    let server = database
        .serve_models(
            true,
            if configured {
                vec![deployment("http://127.0.0.1:1/v1".into())]
            } else {
                vec![]
            },
        )
        .await;
    let worker = server.worker.as_ref().unwrap();
    let workers = worker.repository.workers().await.unwrap();
    assert_eq!(workers.len(), 1);
    let pending = create_lens(
        serde_json::from_value(json!({"name":"Worker eligibility","model":"test-analysis","checks":[{"id":"correct","instruction":"Match the evidence"}]})).unwrap(),
        workers[0].scope.clone(), chrono::Utc::now(), "pending-worker-fixture", "pending-job-fixture"
    ).unwrap();
    worker.repository.create(&pending).await.unwrap();
    if revoked {
        worker
            .repository
            .revoke_worker(&workers[0].id)
            .await
            .unwrap();
    }
    let result = worker.claim().await;
    if unauthorized {
        assert!(matches!(
            result,
            Err(litellm_lens::Error::Control { status: 401, .. })
        ));
    } else {
        assert_eq!(result.unwrap().is_some(), configured);
    }
}

async fn seed_unavailable_page(
    repository: &litellm_storage_clickhouse::investigations::Investigations,
) {
    use lens_investigations::{LensRepository, create_lens};
    let now = chrono::Utc::now() - chrono::TimeDelta::hours(1);
    for index in 0..21 {
        let model = if index == 20 {
            "test-analysis"
        } else {
            "removed-model"
        };
        let record = create_lens(
            serde_json::from_value(json!({"name":"Paged eligibility","model":model,"checks":[{"id":"correct","instruction":"Match the evidence"}]})).unwrap(),
            lens_contract::investigations::Scope {all_teams:true, ..Default::default()},
            now+chrono::TimeDelta::seconds(index), &format!("page-{index:02}"), &format!("job-{index:02}")
        ).unwrap();
        repository.create(&record).await.unwrap();
    }
}

#[rstest]
#[tokio::test]
async fn unavailable_models_on_a_full_page_do_not_starve_other_investigations(
    #[future(awt)] database: Database,
) {
    let server = database
        .serve_models(true, vec![deployment("http://127.0.0.1:1/v1".into())])
        .await;
    let worker = server.worker.as_ref().unwrap();
    seed_unavailable_page(&worker.repository).await;
    let claimed = worker.claim().await.unwrap().unwrap();
    assert_eq!(claimed.lens_id, "page-20");
    assert_eq!(claimed.job.id, "job-20");
    assert_eq!(claimed.job.attempts, 1);
    assert!(worker.claim().await.unwrap().is_none());
}

#[rstest]
#[tokio::test]
async fn background_service_runs_queued_investigations_without_an_http_trigger(
    #[future(awt)] database: Database,
) {
    use lens_investigations::{LensRepository, create_lens};
    let server = database
        .serve_models(true, vec![deployment("http://127.0.0.1:1/v1".into())])
        .await;
    let worker = server.worker.as_ref().unwrap();
    let pending = create_lens(
        serde_json::from_value(json!({"name":"Automatic processing","model":"test-analysis","enabled":false,"checks":[{"id":"correct","instruction":"Match the evidence"}]})).unwrap(),
        lens_contract::investigations::Scope { all_teams:true, ..Default::default() },
        chrono::Utc::now(), "background-fixture", "background-job-fixture"
    ).unwrap();
    worker.repository.create(&pending).await.unwrap();
    let task = tokio::spawn(worker.clone().serve());
    let completed = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let record = worker.repository.get(&pending.id).await.unwrap().unwrap();
            if record.jobs[0].finished_at.is_some() {
                break record;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await;
    task.abort();
    let completed =
        completed.expect("background processing did not finish the queued investigation");
    assert_eq!(completed.jobs[0].status, JobStatus::Completed);
    assert_eq!(completed.jobs[0].attempts, 1);
    assert_eq!(completed.jobs[0].coverage.selected, 0);
    assert_eq!(completed.spent, 0.0);
}

#[rstest]
#[case::no_configured_models(false, "test-analysis", 400)]
#[case::unknown_model(true, "not-configured", 400)]
#[case::configured_model(true, "test-analysis", 200)]
#[tokio::test]
async fn investigation_creation_rejects_unavailable_analysis_models(
    #[future(awt)] database: Database,
    #[case] configured: bool,
    #[case] model: &str,
    #[case] status: u16,
) {
    let server = database
        .serve_models(
            true,
            if configured {
                vec![deployment("http://127.0.0.1:1/v1".into())]
            } else {
                vec![]
            },
        )
        .await;
    let response = reqwest::Client::new().post(format!("{}/lens", server.url)).bearer_auth(ADMIN)
        .json(&json!({"name":"Model setup validation","model":model,"checks":[{"id":"correct","instruction":"Match the evidence"}]}))
        .send().await.unwrap();
    assert_eq!(response.status().as_u16(), status);
    let result: Value = response.json().await.unwrap();
    if status == 200 {
        assert_eq!(result["settings"]["model"], "test-analysis");
    } else {
        assert!(
            result
                .to_string()
                .contains("Choose a configured Lens analysis model")
        );
    }
}
