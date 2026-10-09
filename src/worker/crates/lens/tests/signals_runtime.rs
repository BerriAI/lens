mod datasets {
    pub mod support;
}

use chrono::Utc;
use datasets::support::{Database, INGEST, database};
use lens_contract::{
    feedback::TraceIdentity,
    investigations::Scope,
    signals::{SignalConfig, SignalData, SignalEvidence, TraceSignalStatus},
};
use lens_decisions::{Deployment, EvaluationModels, Provider, Secret, TransportLimits};
use lens_signals::{LIVE_SWEEP, SignalReader, SignalRepository, run_signal_tick, trace_signals};
use litellm_storage_clickhouse::signals::Signals;
use rstest::rstest;
use serde_json::{Value, json};
use std::time::Duration;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[rstest]
#[case::classified(200, TraceSignalStatus::Classified)]
#[case::provider_failed(503, TraceSignalStatus::Failed)]
#[tokio::test]
async fn ingested_trace_is_classified_and_durable_without_a_gateway(
    #[future(awt)] database: Database,
    #[case] status: u16,
    #[case] expected: TraceSignalStatus,
) {
    let server = database.serve(false).await;
    let ingested_at = Utc::now();
    let now = ingested_at + chrono::TimeDelta::seconds(30);
    let at = (ingested_at - chrono::TimeDelta::seconds(60))
        .timestamp_nanos_opt()
        .unwrap();
    let payload = json!({"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"signal-agent"}}]},"scopeSpans":[{"spans":[{"traceId":"a1112222333344445555666677778888","spanId":"1111222233334444","name":"Signal agent","startTimeUnixNano":at.to_string(),"endTimeUnixNano":(at+1_000_000).to_string(),"attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"invoke_agent"}},{"key":"gen_ai.agent.name","value":{"stringValue":"signal-agent"}},{"key":"gen_ai.input.messages","value":{"stringValue":"[{\"role\":\"user\",\"content\":\"I am frustrated that you ignored the request\"}]"}}],"status":{"code":1}}]}]}]});
    let result = reqwest::Client::new()
        .post(format!("{}/v1/traces", server.url))
        .bearer_auth(INGEST)
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert_eq!(result.status(), 200, "{}", result.text().await.unwrap());
    let provider = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(
            ResponseTemplate::new(status).set_body_json(json!({"answers":{
                "frustration":{"type":"noul","noul":0.9},
                "__evidence_0":{"type":"choice","choice":"L000","confidence":0.95}
            }})),
        )
        .expect(1)
        .mount(&provider)
        .await;
    let models = EvaluationModels::new(
        lens_analysis::bundled_catalog().unwrap(),
        vec![Deployment {
            name: "signals".into(),
            model: "typesafe/jev-latest".into(),
            provider: Provider::Typesafe,
            api_base: Some(provider.uri().parse().unwrap()),
            api_key: Some(Secret::new("synthetic-key")),
        }],
        TransportLimits {
            timeout: Duration::from_secs(5),
            max_response_bytes: 65536,
        },
    )
    .unwrap();
    let config:SignalConfig=serde_json::from_value(json!({"model":"signals","threshold":0.5,"signals":[{"id":"frustration","name":"Frustration","question":"Is the user frustrated?"}]})).unwrap();
    let repository = Signals(server.store.clone());
    repository.save_config(&config).await.unwrap();
    let settling = run_signal_tick(
        &server.sources,
        Some(&repository),
        Some(&models),
        &|| ingested_at,
        true,
        "",
        LIVE_SWEEP,
    )
    .await
    .unwrap();
    assert_eq!(settling.claimed, 0);
    assert!(provider.received_requests().await.unwrap().is_empty());
    let tick = run_signal_tick(
        &server.sources,
        Some(&repository),
        Some(&models),
        &|| now,
        true,
        "",
        LIVE_SWEEP,
    )
    .await
    .unwrap();
    assert_eq!(tick.claimed, 1);
    let scope = Scope {
        all_teams: true,
        ..Default::default()
    };
    let sample = SignalReader::sample(
        &server.sources,
        &scope,
        (now - chrono::TimeDelta::minutes(15)).timestamp_millis(),
        now.timestamp_millis(),
        100,
        "",
    )
    .await
    .unwrap();
    let execution = &sample.executions[0];
    let content = SignalReader::content(&server.sources, &scope, execution, "")
        .await
        .unwrap();
    assert_eq!(content.parts.len(), 1);
    let evidence = SignalEvidence {
        span_id: "1111222233334444".into(),
        quote: content.parts[0].content.trim().into(),
    };
    assert_eq!(content.parts[0].span_id, evidence.span_id);
    assert!(
        evidence
            .quote
            .contains("I am frustrated that you ignored the request")
    );
    let identity = TraceIdentity {
        trace_id: execution.trace_id.clone(),
        trace_ref: execution.trace_ref.clone(),
    };
    let reopened = Signals(server.store.clone());
    let rows = reopened
        .traces(std::slice::from_ref(&identity))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].claimed_until.is_none());
    assert_eq!(rows[0].classified_at, Some(now));
    let projected = trace_signals(&identity, rows.first(), &config).unwrap();
    assert_eq!(projected.status, expected);
    assert_eq!(projected.model, "signals");
    assert_eq!(projected.flags.len(), usize::from(status == 200));
    let stored: SignalData = serde_json::from_value(rows[0].data.clone()).unwrap();
    if status == 200 {
        assert_eq!(stored.scores["frustration"], 0.9);
        assert_eq!(stored.evidence.get("frustration"), Some(&evidence));
        assert_eq!(projected.flags[0].signal_id, "frustration");
        assert_eq!(projected.flags[0].score, 0.9);
        assert_eq!(projected.flags[0].evidence.as_ref(), Some(&evidence));
    } else {
        assert!(stored.scores.is_empty());
        assert!(stored.evidence.is_empty());
    }
    let requests = provider.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let sent: Value = requests[0].body_json().unwrap();
    assert_eq!(sent["model"], "jev-latest");
    assert_eq!(
        sent["state"]["steps"][0]["content"],
        format!("[L000] {}", evidence.quote)
    );
    assert_eq!(sent["questions"].as_object().unwrap().len(), 2);
    assert_eq!(
        sent["questions"]["frustration"],
        json!({"type":"noul","instructions":"Is the user frustrated?"})
    );
    assert_eq!(sent["questions"]["__evidence_0"]["type"], "choice");
    assert!(
        sent["questions"]["__evidence_0"]["instructions"]
            .as_str()
            .unwrap()
            .contains("Is the user frustrated?")
    );
    assert_eq!(
        sent["questions"]["__evidence_0"]["criteria"],
        json!({"L000":null,"none":"No excerpt directly supports a yes answer"})
    );
    assert_eq!(
        run_signal_tick(
            &server.sources,
            Some(&reopened),
            Some(&models),
            &|| now,
            true,
            "",
            LIVE_SWEEP
        )
        .await
        .unwrap()
        .claimed,
        0
    );
    assert_eq!(provider.received_requests().await.unwrap().len(), 1);
}
