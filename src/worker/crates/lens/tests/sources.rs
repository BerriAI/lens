mod datasets {
    pub mod support;
}

use datasets::support::{Database, INGEST, database};
use lens_contract::{
    execution::ExecutionId,
    investigations::Scope,
    worker::{Evidence, LensSettings},
};
use litellm_lens::SampleRequest;
use rstest::{fixture, rstest};
use serde_json::{Value, json};

#[fixture]
fn payload() -> Value {
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap();
    json!({"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"source-agent"}}]},"scopeSpans":[{"spans":[{
        "traceId":"11112222333344445555666677778888", "spanId":"1111222233334444",
        "name":"Source agent", "startTimeUnixNano":now.to_string(), "endTimeUnixNano":(now+1_000_000).to_string(),
        "attributes":[
            {"key":"gen_ai.operation.name","value":{"stringValue":"invoke_agent"}},
            {"key":"gen_ai.agent.name","value":{"stringValue":"source-agent"}},
            {"key":"gen_ai.input.messages","value":{"stringValue":"[{\"role\":\"user\",\"content\":\"Find the missing invoice\"}]"}},
            {"key":"customer.segment","value":{"stringValue":"test"}}
        ], "status":{"code":1}
    }]}]}]})
}

#[rstest]
#[case::admin(Scope {all_teams:true, ..Default::default()}, true)]
#[case::team(Scope {team_id:"dataset-team".into(), ..Default::default()}, true)]
#[case::key(Scope {team_id:"dataset-team".into(), api_key_hash:"dataset-key-hash".into(), ..Default::default()}, true)]
#[case::other_team(Scope {team_id:"unrelated-team".into(), ..Default::default()}, false)]
#[case::other_key(Scope {team_id:"dataset-team".into(), api_key_hash:"unrelated-key".into(), ..Default::default()}, false)]
#[tokio::test]
async fn source_sampling_and_evidence_preserve_scopes(
    #[future(awt)] database: Database,
    payload: Value,
    #[case] scope: Scope,
    #[case] visible: bool,
) {
    let server = database.serve(false).await;
    let response = reqwest::Client::new()
        .post(format!("{}/v1/traces", server.url))
        .bearer_auth(INGEST)
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
    let settings: LensSettings = serde_json::from_value(
        json!({"name":"Source test","model":"test-model","agent_name":"source-agent"}),
    )
    .unwrap();
    let selection = (&settings).into();
    let availability = server.sources.availability(&scope).await.unwrap();
    assert_eq!(availability.traces != 0, visible);
    assert_eq!(
        server.sources.agents(&scope).await.unwrap(),
        if visible {
            vec!["source-agent".to_owned()]
        } else {
            vec![]
        }
    );
    let sample = server
        .sources
        .sample(
            &scope,
            SampleRequest {
                selection: &selection,
                start: 0,
                end: chrono::Utc::now().timestamp_millis() as u64 + 1000,
                offset: 0,
                page_size: 100,
                preview: false,
                cursor: "",
            },
        )
        .await
        .unwrap();
    assert_eq!(sample.eligible, i64::from(visible));
    assert_eq!(sample.selected, i64::from(visible));
    assert_eq!(sample.executions.len(), usize::from(visible));
    assert!(sample.next_cursor.is_none());
    assert!(sample.next_offset.is_none());
    if !visible {
        return;
    }
    let execution = &sample.executions[0];
    assert_eq!(execution.trace_id, "11112222333344445555666677778888");
    assert_eq!(execution.team_id, "dataset-team");
    assert!(!execution.trace_ref.is_empty());
    assert_eq!(
        ExecutionId::decode(&execution.id).unwrap().trace_ref,
        execution.trace_ref
    );
    assert!(
        !execution
            .metadata
            .iter()
            .any(|field| field.key.as_str() == "litellm.api_key_hash")
    );
    assert!(execution.metadata.iter().any(|field|field.key.as_str()=="customer.segment" && field.value.as_str()=="test"));
    let content = server
        .sources
        .content(&scope, execution, "", Some(0))
        .await
        .unwrap();
    assert_eq!(content.parts.len(), 1);
    assert!(
        content.parts[0]
            .content
            .contains("Find the missing invoice")
    );
    assert!(!content.partial);
    assert!(content.next_cursor.is_none());
    let quote:Evidence = serde_json::from_value(json!({"execution_id":execution.id,"span_id":content.parts[0].span_id,"quote":"Find the missing invoice"})).unwrap();
    assert!(
        server
            .sources
            .verify_evidence(&scope, execution, &quote)
            .await
            .unwrap()
    );
    let invalid = Evidence {
        quote: "Invented quote".try_into().unwrap(),
        ..quote
    };
    assert!(
        !server
            .sources
            .verify_evidence(&scope, execution, &invalid)
            .await
            .unwrap()
    );
    let denied = Scope {
        team_id: "unrelated-team".into(),
        ..Default::default()
    };
    assert!(
        server
            .sources
            .content(&denied, execution, "", Some(0))
            .await
            .unwrap()
            .parts
            .is_empty()
    );
    assert!(
        !server
            .sources
            .verify_evidence(&denied, execution, &invalid)
            .await
            .unwrap()
    );
}
