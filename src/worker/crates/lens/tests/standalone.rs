mod datasets {
    pub mod support;
}

use datasets::support::{ADMIN, Database, database};
use lens_contract::{
    datasets::Dataset,
    eval::{EvalRun, RunStatus},
    ingestion::IngestionKeyCreated,
};
use rstest::{fixture, rstest};
use serde_json::{Value, json};

const GATEWAY_SECRET: &str = "standalone-eval-signing-secret-at-least-32";

fn gateway_identity(role: &str, team: &str) -> String {
    let now = chrono::Utc::now().timestamp();
    jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &json!({
            "iss":"litellm","aud":"litellm-lens","sub":"eval-user","iat":now,"exp":now+60,
            "identity":{"user_role":role,"user_id":"eval-user","team_id":team,"token":"eval-key"}
        }),
        &jsonwebtoken::EncodingKey::from_secret(GATEWAY_SECRET.as_bytes()),
    )
    .unwrap()
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

#[rstest]
#[tokio::test]
async fn standalone_bootstrap_keeps_dataset_and_eval_routes_together(
    #[future(awt)] database: Database,
) {
    let server = database
        .serve_with_gateway(true, Some(GATEWAY_SECRET.into()))
        .await;
    let client = reqwest::Client::new();
    let admin = gateway_identity("proxy_admin", "eval-team");
    let team = gateway_identity("team", "eval-team");
    let other_team = gateway_identity("team", "other-team");
    let service = client
        .get(format!("{}/lens/service", server.url))
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(service.status(), 200);
    assert_eq!(service.json::<Value>().await.unwrap()["connected"], true);

    let created = client
        .post(format!("{}/lens/datasets", server.url))
        .bearer_auth(&admin)
        .json(&json!({"name":"bootstrap-eval","agent_name":"bootstrap-agent"}))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 200, "{}", created.text().await.unwrap());
    let dataset: Dataset = created.json().await.unwrap();
    let saved = client
        .post(format!(
            "{}/lens/datasets/{}/revisions",
            server.url, dataset.id
        ))
        .bearer_auth(&admin)
        .json(&json!({"base_revision":0,"cases":[{
            "id":"case-a","messages":[{"role":"user","content":"Run tests"}],
            "expected":"Tests pass","source":{}
        }]}))
        .send()
        .await
        .unwrap();
    assert_eq!(saved.status(), 200, "{}", saved.text().await.unwrap());
    let saved: Dataset = saved.json().await.unwrap();
    let case_id = &saved.cases[0].id;
    let resolved = client
        .get(format!("{}/lens/datasets/resolve", server.url))
        .query(&[("name", "bootstrap-eval")])
        .bearer_auth(&team)
        .header("X-Lens-Contract", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(resolved.status(), 200);
    assert_eq!(resolved.json::<Value>().await.unwrap()["id"], dataset.id);
    let cases_url = format!(
        "{}/lens/datasets/{}/revisions/{}/cases",
        server.url, dataset.id, saved.revision
    );
    let cases = client
        .get(&cases_url)
        .bearer_auth(&team)
        .header("X-Lens-Contract", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(cases.status(), 200, "{}", cases.text().await.unwrap());
    assert_eq!(
        cases.json::<Value>().await.unwrap()["cases"][0]["id"],
        *case_id
    );
    let hidden = client
        .get(&cases_url)
        .bearer_auth(&other_team)
        .header("X-Lens-Contract", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(hidden.status(), 404);

    let created = client
        .post(format!("{}/lens/evals/runs", server.url))
        .bearer_auth(&team)
        .header("X-Lens-Contract", "1")
        .header("Idempotency-Key", "bootstrap-run")
        .json(&json!({
            "eval":"bootstrap-eval","agent":"bootstrap-agent","dataset_id":dataset.id,
            "revision":saved.revision,"version":"test","branch":"main","trials":1,
            "scorers":[{"kind":"task_completed"}]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 201, "{}", created.text().await.unwrap());
    let run: EvalRun = created.json().await.unwrap();
    let run_url = format!("{}/lens/evals/runs/{}", server.url, run.id);
    let result = client
        .put(format!("{run_url}/results/{case_id}/0"))
        .bearer_auth(&team)
        .header("X-Lens-Contract", "1")
        .json(&json!({"error":{"type":"ValueError","message":"agent failed"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(result.status(), 204);
    let read = client
        .get(&run_url)
        .bearer_auth(&team)
        .header("X-Lens-Contract", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(read.status(), 200);
    assert_eq!(read.json::<EvalRun>().await.unwrap().received_trials, 1);
    let hidden = client
        .get(&run_url)
        .bearer_auth(&other_team)
        .header("X-Lens-Contract", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(hidden.status(), 404);
    let listed = client
        .get(format!("{}/lens/evals/runs", server.url))
        .bearer_auth(&team)
        .header("X-Lens-Contract", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(listed.status(), 200);
    assert_eq!(listed.json::<Vec<EvalRun>>().await.unwrap()[0].id, run.id);
    let finished = client
        .post(format!("{run_url}/finish"))
        .bearer_auth(&team)
        .header("X-Lens-Contract", "1")
        .send()
        .await
        .unwrap();
    assert_eq!(finished.status(), 202, "{}", finished.text().await.unwrap());
    assert_eq!(
        finished.json::<EvalRun>().await.unwrap().status,
        RunStatus::Scoring
    );
}
