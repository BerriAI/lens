mod datasets {
    pub mod support;
}

use datasets::support::{ADMIN, Database, database};
use lens_contract::ingestion::IngestionKeyCreated;
use rstest::{fixture, rstest};
use serde_json::{Value, json};

async fn eval_request(
    client: &reqwest::Client,
    url: &str,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> Value {
    let mut request = client
        .request(method, format!("{url}{path}"))
        .bearer_auth(ADMIN)
        .header("X-Lens-Contract", "1");
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.unwrap();
    let status = response.status();
    let bytes = response.bytes().await.unwrap();
    assert!(
        status.is_success(),
        "{path}: {}",
        String::from_utf8_lossy(&bytes)
    );
    if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    }
}

#[rstest]
#[case::trace_id("trace_id", "11112222333344445555666677778888")]
#[case::session_id("session.id", "standalone-eval-session")]
#[tokio::test]
async fn standalone_default_scope_closes_ingested_evals_without_a_gateway(
    #[future(awt)] database: Database,
    mut payload: Value,
    #[case] attribute: &str,
    #[case] reference: &str,
) {
    let server = database.serve(true).await;
    let client = reqwest::Client::new();
    let key: IngestionKeyCreated = serde_json::from_value(
        eval_request(
            &client,
            &server.url,
            reqwest::Method::POST,
            "/lens/tracing/keys",
            Some(json!({"name":"Standalone evaluation", "team_id":""})),
        )
        .await,
    )
    .unwrap();
    payload["resourceSpans"][0]["resource"]["attributes"] = json!([
        {"key":"agent.name","value":{"stringValue":"standalone-agent"}},
        {"key":"agent.version","value":{"stringValue":"standalone-build"}},
        {"key":"deployment.environment","value":{"stringValue":"lens-eval"}},
        {"key":"session.id","value":{"stringValue":"standalone-eval-session"}},
        {"key":"repo_url","value":{"stringValue":"https://example.test/agent"}}
    ]);
    let ingested = client
        .post(format!("{}/v1/traces", server.url))
        .bearer_auth(&key.key)
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert_eq!(ingested.status(), 200, "{}", ingested.text().await.unwrap());
    let dataset = eval_request(
        &client,
        &server.url,
        reqwest::Method::POST,
        "/lens/datasets",
        Some(json!({"name":"Standalone eval cases", "agent_name":"standalone-agent"})),
    )
    .await;
    let id = dataset["id"].as_str().unwrap();
    eval_request(
        &client,
        &server.url,
        reqwest::Method::POST,
        &format!("/lens/datasets/{id}/revisions"),
        Some(json!({"base_revision":0,"cases":[{
            "id":"first", "messages":[{"role":"user","content":"Finish the task"}],
            "source":{"trace_id":"11112222333344445555666677778888"}
        }]})),
    )
    .await;
    let cases = eval_request(
        &client,
        &server.url,
        reqwest::Method::GET,
        &format!("/lens/datasets/{id}/revisions/1/cases"),
        None,
    )
    .await;
    assert_eq!(
        cases["cases"][0]["meta"]["repo_url"],
        "https://example.test/agent"
    );
    let case_id = cases["cases"][0]["id"].as_str().unwrap();
    let run = eval_request(
        &client,
        &server.url,
        reqwest::Method::POST,
        "/lens/evals/runs",
        Some(json!({
            "eval":"standalone-eval", "agent":"standalone-agent", "dataset_id":id,
            "revision":1, "version":"standalone-build", "branch":"main",
            "trials":1, "scorers":[{"kind":"task_completed"}], "gate":{"pass_rate":1.0}
        })),
    )
    .await;
    let run_id = run["id"].as_str().unwrap();
    eval_request(
        &client,
        &server.url,
        reqwest::Method::PUT,
        &format!("/lens/evals/runs/{run_id}/results/{case_id}/0"),
        Some(json!({"trace":{"attribute":attribute,"value":reference}})),
    )
    .await;
    eval_request(
        &client,
        &server.url,
        reqwest::Method::POST,
        &format!("/lens/evals/runs/{run_id}/finish"),
        None,
    )
    .await;
    let completed = eval_request(
        &client,
        &server.url,
        reqwest::Method::GET,
        &format!("/lens/evals/runs/{run_id}?wait=10"),
        None,
    )
    .await;
    assert_eq!(completed["status"], "done", "{completed}");
    assert_eq!(completed["summary"]["passed"], 1);
    assert_eq!(completed["summary"]["errors"], 0);
    assert_eq!(completed["summary"]["gate"]["passed"], true);
    let detail = eval_request(
        &client,
        &server.url,
        reqwest::Method::GET,
        &format!("/lens/evals/runs/{run_id}/cases/{case_id}"),
        None,
    )
    .await;
    assert_eq!(detail["passed"], true);
    assert_eq!(detail["trials"][0]["checks"][0]["passed"], true);
    assert_eq!(detail["trials"][0]["error"], Value::Null);
}

#[fixture]
fn payload() -> Value {
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap();
    json!({"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"standalone-agent"}}]},"scopeSpans":[{"spans":[{
        "traceId":"11112222333344445555666677778888", "spanId":"1111222233334444",
        "name":"Standalone agent", "startTimeUnixNano":now.to_string(), "endTimeUnixNano":(now+1_000_000).to_string(),
        "attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"invoke_agent"}},{"key":"gen_ai.agent.name","value":{"stringValue":"standalone-agent"}}],
        "status":{"code":1}
    }]}]}]})
}

#[rstest]
#[tokio::test]
async fn standalone_owns_sessions_keys_and_traces_across_restart(
    #[future(awt)] database: Database,
    payload: Value,
) {
    let server = database.serve(true).await;
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
    let created = client
        .post(format!("{}/lens/tracing/keys", server.url))
        .header("cookie", &cookie)
        .header("origin", &server.url)
        .json(&json!({"name":"Standalone sender", "team_id":"standalone-team"}))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 200, "{}", created.text().await.unwrap());
    let created: IngestionKeyCreated = created.json().await.unwrap();
    assert!(created.active);
    let snapshot = lens_auth::ingestion::Ingestion(
        litellm_storage_clickhouse::ingestion::IngestionKeys(server.store.clone()),
    )
    .current_snapshot(chrono::Utc::now())
    .await
    .unwrap();
    assert_eq!(snapshot.keys.len(), 1);
    assert_eq!(snapshot.keys[0].tenant.team_id, "standalone-team");
    assert_ne!(snapshot.keys[0].token_hash, created.key);
    let ingested = client
        .post(format!("{}/v1/traces", server.url))
        .bearer_auth(&created.key)
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert_eq!(ingested.status(), 200, "{}", ingested.text().await.unwrap());
    let list = client
        .get(format!("{}/v1/traces", server.url))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), 200);
    let listed: Value = list.json().await.unwrap();
    assert_eq!(listed["data"].as_array().unwrap().len(), 1);
    let trace_ref = listed["data"][0]["trace_ref"].as_str().unwrap();
    drop(server);
    let restarted = database.serve(true).await;
    let session = client
        .get(format!("{}/auth/session", restarted.url))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(session.status(), 200);
    let detail = client
        .get(format!(
            "{}/v1/traces/11112222333344445555666677778888",
            restarted.url
        ))
        .query(&[("trace_ref", trace_ref)])
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(detail.status(), 200);
    let detail: Value = detail.json().await.unwrap();
    assert_eq!(
        detail["summary"]["trace_id"],
        "11112222333344445555666677778888"
    );
    assert_eq!(detail["spans"].as_array().unwrap().len(), 1);
    let active = client
        .post(format!("{}/v1/traces", restarted.url))
        .bearer_auth(&created.key)
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert_eq!(active.status(), 200);
    let revoked = client
        .delete(format!(
            "{}/lens/tracing/keys/{}",
            restarted.url, created.record.id
        ))
        .header("cookie", &cookie)
        .header("origin", &restarted.url)
        .send()
        .await
        .unwrap();
    assert_eq!(revoked.status(), 200);
    let denied = client
        .post(format!("{}/v1/traces", restarted.url))
        .bearer_auth(&created.key)
        .json(&payload)
        .send()
        .await
        .unwrap();
    let status = denied.status();
    assert!(matches!(status.as_u16(), 401 | 429));
    let code = if status.as_u16() == 429 {
        assert_eq!(denied.headers()["retry-after"], "5");
        8
    } else {
        16
    };
    assert_eq!(
        denied.json::<Value>().await.unwrap(),
        json!({"code":code, "message":status.canonical_reason().unwrap()})
    );
    let logout = client
        .delete(format!("{}/auth/session", restarted.url))
        .header("cookie", &cookie)
        .header("origin", &restarted.url)
        .send()
        .await
        .unwrap();
    assert_eq!(logout.status(), 204);
    let denied = client
        .get(format!("{}/auth/session", restarted.url))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401);
}

#[rstest]
#[tokio::test]
async fn standalone_router_has_public_read_contract_and_rejects_internal_or_unsafe_access(
    #[future(awt)] database: Database,
    payload: Value,
) {
    let server = database.serve(true).await;
    let client = reqwest::Client::new();
    let health = client
        .get(format!("{}/health/ready", server.url))
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), 200);
    let service = client
        .get(format!("{}/lens/service", server.url))
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(service.status(), 200);
    let service: Value = service.json().await.unwrap();
    assert_eq!(service["connected"], true);
    assert_eq!(service["status"]["storage_ready"], true);
    assert_eq!(service["status"]["credentials_ready"], true);
    let absent = client
        .get(format!("{}/internal/status", server.url))
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(absent.status(), 404);
    let denied = client
        .get(format!("{}/v1/traces", server.url))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 401);
    let admin_ingest = client
        .post(format!("{}/v1/traces", server.url))
        .bearer_auth(ADMIN)
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert_eq!(admin_ingest.status(), 401);
    let sql = client
        .post(format!("{}/v1/traces/query", server.url))
        .bearer_auth(ADMIN)
        .json(&json!({"sql":"SELECT 7 AS result"}))
        .send()
        .await
        .unwrap();
    assert_eq!(sql.status(), 200, "{}", sql.text().await.unwrap());
    assert_eq!(
        sql.json::<Value>().await.unwrap(),
        json!({"data":[{"result":7}]})
    );
    let login = client
        .post(format!("{}/auth/session", server.url))
        .json(&json!({"token":ADMIN}))
        .send()
        .await
        .unwrap();
    let cookie = login.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let csrf = client
        .post(format!("{}/lens/tracing/keys", server.url))
        .header("cookie", &cookie)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(csrf.status(), 403);
}
