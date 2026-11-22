mod datasets {
    pub mod support;
}
use chrono::Utc;
use datasets::support::{ADMIN, Database, database};
use lens_auth::{Authentication, Settings};
use litellm_storage_clickhouse::sessions::Sessions;
use rstest::rstest;
use serde_json::{Value, json};
use std::sync::Arc;

#[rstest]
#[tokio::test]
async fn rules_can_be_added_edited_disabled_and_removed(#[future(awt)] database: Database) {
    let server = database.serve(true).await;
    let client = reqwest::Client::new();
    let url = format!("{}/lens/admin/alerts/rules", server.url);
    let rules = json!([
        {"id":"short","enabled":true,"condition":{"kind":"run_duration","minutes":5}},
        {"id":"long","enabled":true,"condition":{"kind":"run_duration","minutes":20}},
        {"id":"errors","enabled":true,"condition":{"kind":"model_error"}}
    ]);
    let saved: Value = client
        .put(&url)
        .bearer_auth(ADMIN)
        .json(&json!({"revision":0,"rules":rules}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let loaded: Value = client
        .get(&url)
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(loaded, saved);
    assert_eq!(loaded["rules"], rules);
    let duplicate = client
        .put(&url)
        .bearer_auth(ADMIN)
        .json(&json!({"revision":saved["revision"],"rules":[rules[0],rules[0]]}))
        .send()
        .await
        .unwrap();
    assert_eq!(duplicate.status(), 422);
    let edited = json!([
        {"id":"short","enabled":false,"condition":{"kind":"run_duration","minutes":5}},
        {"id":"long","enabled":true,"condition":{"kind":"run_duration","minutes":30}}
    ]);
    let updated: Value = client
        .put(&url)
        .bearer_auth(ADMIN)
        .json(&json!({"revision":saved["revision"],"rules":edited}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["rules"], edited);
    assert_ne!(updated["revision"], saved["revision"]);
    let loaded: Value = client
        .get(&url)
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(loaded, updated);
}

#[rstest]
#[tokio::test]
async fn model_error_flows_from_ingestion_to_alert_without_a_model(
    #[future(awt)] database: Database,
) {
    let server = database.serve(true).await;
    let client = reqwest::Client::new();
    let rules_url = format!("{}/lens/admin/alerts/rules", server.url);
    let saved = client
        .put(&rules_url)
        .bearer_auth(ADMIN)
        .json(&json!({"revision":0,"rules":[{"id":"duration","enabled":true,"condition":{"kind":"run_duration","minutes":5}},{"id":"errors","enabled":true,"condition":{"kind":"model_error"}}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(saved.status(), 200);
    let saved: Value = saved.json().await.unwrap();
    assert_eq!(saved["rules"][1]["enabled"], true);
    assert_eq!(
        client
            .put(&rules_url)
            .bearer_auth(ADMIN)
            .json(&json!({"revision":0,"rules":[]}))
            .send()
            .await
            .unwrap()
            .status(),
        409
    );
    assert_eq!(
        client
            .put(&rules_url)
            .bearer_auth(ADMIN)
            .json(
                &json!({"revision":saved["revision"],"rules":[{"id":"duration","enabled":true,"condition":{"kind":"run_duration","minutes":0}}]})
            )
            .send()
            .await
            .unwrap()
            .status(),
        422
    );
    assert_eq!(client.get(&rules_url).send().await.unwrap().status(), 401);
    let key: Value = client
        .post(format!("{}/lens/admin/tracing/keys", server.url))
        .bearer_auth(ADMIN)
        .json(&json!({"name":"Rule test"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let at = Utc::now().timestamp_nanos_opt().unwrap();
    let payload = json!({"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"rule-test"}}]},"scopeSpans":[{"spans":[{"traceId":"a1112222333344445555666677778888","spanId":"1111222233334444","name":"Failed model call","startTimeUnixNano":at.to_string(),"endTimeUnixNano":(at+1_000_000).to_string(),"attributes":[{"key":"gen_ai.operation.name","value":{"stringValue":"chat"}}],"status":{"code":2}}]}]}]});
    let ingest = client
        .post(format!("{}/v1/traces", server.url))
        .bearer_auth(key["key"].as_str().unwrap())
        .json(&payload)
        .send()
        .await
        .unwrap();
    assert_eq!(ingest.status(), 200);
    let authentication = Arc::new(Authentication {
        settings: Settings::new(ADMIN, None, &server.url).unwrap(),
        sessions: Sessions(server.store.clone()),
    });
    let rules = litellm_lens::alert_rules::AlertRules::new(authentication, server.sources.clone());
    let task = tokio::spawn(rules.serve());
    let found = tokio::time::timeout(std::time::Duration::from_secs(45), async {
        loop {
            let overview: Value = client
                .get(format!("{rules_url}/overview"))
                .bearer_auth(ADMIN)
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .json()
                .await
                .unwrap();
            if !overview["matches"].as_array().unwrap().is_empty() {
                break overview;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await;
    task.abort();
    let overview = found.unwrap();
    assert_eq!(overview["matches"].as_array().unwrap().len(), 1);
    assert_eq!(overview["matches"][0]["kind"], "model_error");
    assert_eq!(overview["matches"][0]["rule_id"], "errors");
    assert_eq!(overview["matches"][0]["span_id"], "1111222233334444");
    let signals: Value = client
        .get(format!("{}/lens/admin/signals", server.url))
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(signals["model"], "");
}
