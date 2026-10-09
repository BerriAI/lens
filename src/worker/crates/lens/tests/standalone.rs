mod datasets {
    pub mod support;
}

use datasets::support::{ADMIN, Database, database};
use lens_contract::ingestion::IngestionKeyCreated;
use rstest::{fixture, rstest};
use serde_json::{Value, json};

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
