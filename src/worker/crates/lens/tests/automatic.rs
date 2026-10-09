mod datasets {
    pub mod support;
}

use datasets::support::{ADMIN, Database, database};
use lens_analysis::{Deployment, Provider, Secret};
use lens_contract::investigations::LensList;
use lens_inference::{ModelCapacity, OutputLimits};
use lens_investigations::LensRepository;
use rstest::rstest;
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[rstest]
#[tokio::test]
async fn automatic_analysis_waits_for_ten_traces_and_preserves_pauses(
    #[future(awt)] database: Database,
    #[values(false, true)] gateway: bool,
) {
    let provider = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/model_group/info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{
            "model_group":"aaa-discovered-analysis", "mode":"chat",
            "input_cost_per_token":0.000001, "output_cost_per_token":0.000002,
            "max_input_tokens":100000, "max_output_tokens":4096
        }]})))
        .mount(&provider)
        .await;
    let server = database
        .serve_registry(
            true,
            litellm_lens::gateway::Models::new(
                vec![Deployment {
                    name: "exact-gateway-alias".into(),
                    model: "fixture-model".into(),
                    provider: Provider::OpenAiCompatible,
                    api_base: Some("http://127.0.0.1:1/v1".parse().unwrap()),
                    api_key: Secret::new("fixture-key"),
                    input_cost_per_token: Some(0.000001),
                    output_cost_per_token: Some(0.000002),
                    capacity: ModelCapacity {
                        max_input_tokens: Some(100000.try_into().unwrap()),
                        max_output_tokens: Some(4096.try_into().unwrap()),
                    },
                    output_limits: OutputLimits {
                        max_tokens: Some(1024.try_into().unwrap()),
                        ..Default::default()
                    },
                }],
                vec![],
                None,
                gateway.then(|| {
                    litellm_lens::gateway::GatewayConfig::new(
                        &provider.uri(),
                        "fixture-gateway-key".into(),
                    )
                    .unwrap()
                }),
            )
            .unwrap(),
        )
        .await;
    let client = reqwest::Client::new();
    let worker = server.worker.as_ref().unwrap();
    if gateway {
        assert_eq!(worker.models.get().models()[0], "aaa-discovered-analysis");
    }
    worker.reconcile_automatic().await.unwrap();
    assert!(worker.claim().await.unwrap().is_none());
    seed_traces(&server, 1..=9).await;
    worker.reconcile_automatic().await.unwrap();
    assert!(
        worker.claim().await.unwrap().is_none(),
        "nine traces must wait"
    );
    seed_traces(&server, 10..=10).await;
    let list: LensList = client
        .get(format!("{}/lens", server.url))
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.lenses.len(), 1);
    let lens = &list.lenses[0];
    assert!(lens.jobs.is_empty());
    assert_eq!(lens.settings.agent_name, "automatic-test");
    assert_eq!(lens.settings.model.as_str(), "exact-gateway-alias");
    let paused = lens_investigations::update_settings(
        lens,
        lens_contract::worker::LensSettings {
            enabled: false,
            context: "Preserve these instructions".into(),
            interval_minutes: 7.try_into().unwrap(),
            model: if gateway {
                "aaa-discovered-analysis"
            } else {
                "exact-gateway-alias"
            }
            .try_into()
            .unwrap(),
            ..lens.settings.clone()
        },
        chrono::Utc::now(),
    )
    .unwrap();
    let paused = worker.repository.replace(lens, &paused).await.unwrap();
    worker.reconcile_automatic().await.unwrap();
    assert!(worker.claim().await.unwrap().is_none());
    let saved = worker.repository.get(&paused.id).await.unwrap().unwrap();
    assert!(!saved.settings.enabled);
    assert_eq!(saved.settings.context, "Preserve these instructions");
    assert_eq!(saved.settings.interval_minutes.get(), 7);
    assert_eq!(saved.settings.model, paused.settings.model);
    let enabled = lens_investigations::update_settings(
        &saved,
        lens_contract::worker::LensSettings {
            enabled: true,
            ..saved.settings.clone()
        },
        chrono::Utc::now(),
    )
    .unwrap();
    worker.repository.replace(&saved, &enabled).await.unwrap();
    let claim = worker
        .claim()
        .await
        .unwrap()
        .expect("tenth trace should start analysis");
    assert_eq!(claim.lens_id, lens.id);
    assert_eq!(claim.job.settings.context, "Preserve these instructions");
    assert!(
        worker.claim().await.unwrap().is_none(),
        "one active analysis per agent"
    );
}

async fn seed_traces(server: &datasets::support::Server, numbers: std::ops::RangeInclusive<u64>) {
    let at = chrono::Utc::now() - chrono::TimeDelta::minutes(5);
    let rows = numbers.map(|number| json!({
        "Timestamp": at.to_rfc3339_opts(chrono::SecondsFormat::Nanos,true),
        "TraceId": format!("{number:032x}"), "SpanId": format!("{number:016x}"),
        "SpanName": "automatic fixture", "ServiceName": "automatic-test", "AgentName": "automatic-test",
        "TeamId": "fixture-team", "ApiKeyHash": "fixture-key-hash", "Duration": 1_000_000_u64,
        "EngineReceivedMs": at.timestamp_millis() + 1
    }).to_string()).collect::<Vec<_>>().join("\n");
    let storage = &server.sources.0.storage;
    litellm_storage_clickhouse::execute_statement(
        &storage.client,
        storage.config.storage().writer(),
        &format!(
            "INSERT INTO `{}`.otel_traces FORMAT JSONEachRow\n{rows}",
            storage.config.storage().database()
        ),
        std::time::Duration::from_secs(30),
    )
    .await
    .unwrap();
}
