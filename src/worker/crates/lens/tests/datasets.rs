mod datasets {
    pub mod support;
}

use base64::{Engine, engine::general_purpose::URL_SAFE};
use datasets::support::{ADMIN, Database, INGEST, database};
use lens_contract::datasets::{BuildResult, Dataset};
use litellm_lens::auth::unix_seconds;
use litellm_storage_clickhouse::state::Change;
use rstest::{fixture, rstest};
use serde_json::{Value, json};

const TRACE: &str = "01020304050607080102030405060708";

#[derive(Clone, Copy)]
enum Source {
    Trace,
    Span,
    Finding,
}

#[fixture]
fn trace_payload(#[default(2)] count: usize) -> Value {
    let now = unix_seconds() * 1_000_000_000;
    let spans: Vec<_> = (0..count).map(|index| json!({
        "traceId": TRACE, "spanId": format!("{:016x}", index + 1), "name": format!("model call {index}"),
        "startTimeUnixNano": (now + index as u64 * 1_000_000).to_string(),
        "endTimeUnixNano": (now + (index as u64 + 1) * 1_000_000).to_string(),
        "attributes": [
            {"key":"gen_ai.operation.name","value":{"stringValue":"chat"}},
            {"key":"gen_ai.request.model","value":{"stringValue":"fixture-model"}},
            {"key":"gen_ai.input.messages","value":{"stringValue":json!([{"role":"user","content":format!("prompt-{index}")}]).to_string()}},
            {"key":"gen_ai.output.messages","value":{"stringValue":json!([{"role":"assistant","content":format!("reply-{index}")}]).to_string()}},
            {"key":"agent.version","value":{"stringValue":"version-7"}}
        ], "status":{"code":1}
    })).collect();
    json!({"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"dataset-agent"}}]},"scopeSpans":[{"spans":spans}]}]})
}

#[rstest]
#[case::latest_conversation(2, Source::Trace, 1)]
#[case::conversation_after_first_page(502, Source::Trace, 501)]
#[case::explicit_span(3, Source::Span, 0)]
#[case::finding_evidence(3, Source::Finding, 0)]
#[tokio::test]
async fn ingested_traces_build_cases_through_the_application_router(
    #[future(awt)] database: Database,
    #[case] count: usize,
    #[case] source: Source,
    #[case] index: usize,
) {
    let server = database.serve(false).await;
    let client = reqwest::Client::new();
    let ingested = client
        .post(format!("{}/v1/traces", server.url))
        .bearer_auth(INGEST)
        .json(&trace_payload(count))
        .send()
        .await
        .unwrap();
    assert_eq!(ingested.status(), 200, "{}", ingested.text().await.unwrap());
    let source = match source {
        Source::Trace => json!({"kind":"trace","trace_id":TRACE}),
        Source::Span => json!({"kind":"trace","trace_id":TRACE,"span_id":"0000000000000001"}),
        Source::Finding => {
            let execution = URL_SAFE
                .encode(serde_json::to_vec(&json!(["otel", "dataset-team", TRACE, ""])).unwrap());
            let previous = server
                .store
                .read("lens/%5B%22fixture-lens%22%5D")
                .await
                .unwrap();
            server.store.commit(vec![Change {previous, value:json!({"lens":{
                "scope":{"all_teams":false,"team_id":"dataset-team","api_key_hash":""},
                "findings":[{"id":"finding-1","evidence":[{"execution_id":execution,"span_id":"0000000000000001"}]}]
            }})}]).await.unwrap();
            json!({"kind":"finding","lens_id":"fixture-lens","finding_ids":["finding-1"]})
        }
    };
    let response = client
        .post(format!("{}/lens/datasets/build", server.url))
        .bearer_auth(ADMIN)
        .json(&json!({"sources":[source]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200, "{}", response.text().await.unwrap());
    let built: BuildResult = response.json().await.unwrap();
    assert!(built.skipped.is_empty(), "{:?}", built.skipped);
    assert_eq!(built.cases.len(), 1);
    let case = &built.cases[0];
    assert_eq!(case.messages[0].content, format!("prompt-{index}"));
    assert_eq!(case.reply, format!("reply-{index}"));
    assert_eq!(case.agent_version, "version-7");
    assert_eq!(case.source.trace_id, TRACE);
    assert_eq!(case.source.span_id, format!("{:016x}", index + 1));
    if source["kind"] == "finding" {
        assert_eq!(case.source.finding_id, "finding-1");
        assert_eq!(case.source.lens_id, "fixture-lens");
    }
}

#[rstest]
#[tokio::test]
async fn session_and_saved_dataset_survive_application_restart(#[future(awt)] database: Database) {
    let server = database.serve(false).await;
    let client = reqwest::Client::new();
    let login = client
        .post(format!("{}/auth/session", server.url))
        .json(&json!({"token":ADMIN}))
        .send()
        .await
        .unwrap();
    assert_eq!(login.status(), 200);
    let cookie = login.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let create = client
        .post(format!("{}/lens/datasets", server.url))
        .header("cookie", &cookie)
        .header("origin", &server.url)
        .json(&json!({"name":"Saved cases","agent_name":"dataset-agent"}))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), 200, "{}", create.text().await.unwrap());
    let created: Dataset = create.json().await.unwrap();
    let build = client.post(format!("{}/lens/datasets/build", server.url)).bearer_auth(ADMIN)
        .json(&json!({"dataset_id":created.id,"sources":[{"kind":"text","text":"A persisted question"}]})).send().await.unwrap();
    assert_eq!(build.status(), 200);
    let built: BuildResult = build.json().await.unwrap();
    let save = client
        .post(format!(
            "{}/lens/datasets/{}/revisions",
            server.url, created.id
        ))
        .header("cookie", &cookie)
        .header("origin", &server.url)
        .json(&json!({"base_revision":0,"cases":built.cases}))
        .send()
        .await
        .unwrap();
    assert_eq!(save.status(), 200, "{}", save.text().await.unwrap());
    let saved: Dataset = save.json().await.unwrap();
    assert_eq!(saved.revision, 1);
    drop(server);
    let restarted = database.serve(false).await;
    let read = client
        .get(format!("{}/lens/datasets/{}", restarted.url, saved.id))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(read.status(), 200);
    assert_eq!(read.json::<Dataset>().await.unwrap(), saved);
    let exported = client
        .get(format!(
            "{}/lens/datasets/{}/export",
            restarted.url, saved.id
        ))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(exported.status(), 200);
    assert_eq!(exported.headers()["content-type"], "application/x-ndjson");
    assert_eq!(
        exported.text().await.unwrap(),
        format!("{}\n", serde_json::to_string(&saved.cases[0]).unwrap())
    );
}
