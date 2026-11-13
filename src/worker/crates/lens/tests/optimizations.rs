mod datasets {
    pub mod support;
}
use datasets::support::{ADMIN, Database, database};
use litellm_lens::gateway::{GatewayConfig, Models};
use rstest::rstest;
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_partial_json, method, path},
};

async fn api(server: &datasets::support::Server, route: &str, body: Option<Value>) -> Value {
    let client = reqwest::Client::new();
    let request = if let Some(body) = body {
        client.post(format!("{}{route}", server.url)).json(&body)
    } else {
        client.get(format!("{}{route}", server.url))
    };
    let response = request.bearer_auth(ADMIN).send().await.unwrap();
    let status = response.status();
    let body: Value = response.json().await.unwrap();
    assert!(status.is_success(), "{route}: {status} {body}");
    body
}

#[rstest]
#[tokio::test]
async fn offline_replays_both_models_persists_measured_results_and_obeys_pause(
    #[future(awt)] database: Database,
) {
    let gateway = MockServer::start().await;
    Mock::given(method("GET")).and(path("/model_group/info")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[
        {"model_group":"baseline","mode":"chat"},{"model_group":"candidate","mode":"chat"},{"model_group":"judge","mode":"chat"}
    ]}))).mount(&gateway).await;
    Mock::given(method("POST")).and(path("/chat/completions")).respond_with(ResponseTemplate::new(200).insert_header("x-litellm-response-cost","0.001").set_body_json(json!({"model":"candidate","choices":[{"message":{"content":"The answer is 4"},"finish_reason":"stop"}]}))).with_priority(10).mount(&gateway).await;
    Mock::given(method("POST")).and(path("/chat/completions")).and(body_partial_json(json!({"model":"baseline"}))).respond_with(ResponseTemplate::new(200).insert_header("x-litellm-response-cost","0.002").set_body_json(json!({"model":"baseline","choices":[{"message":{"content":"4"},"finish_reason":"stop"}]}))).mount(&gateway).await;
    Mock::given(method("POST")).and(path("/chat/completions")).and(body_partial_json(json!({"model":"judge"}))).respond_with(ResponseTemplate::new(200).set_body_json(json!({"model":"judge","choices":[{"message":{"content":"{\"score\":1.0}"},"finish_reason":"stop"}]}))).mount(&gateway).await;
    let models = || {
        Models::new(
            vec![],
            vec![],
            None,
            Some(GatewayConfig::new(&gateway.uri(), "fixture-only".into()).unwrap()),
        )
        .unwrap()
    };
    let server = database.serve_registry(true, models()).await;
    let dataset = api(
        &server,
        "/lens/datasets",
        Some(json!({"name":"Arithmetic","agent_name":"calculator"})),
    )
    .await;
    let dataset_id = dataset["id"].as_str().unwrap();
    api(&server,&format!("/lens/datasets/{dataset_id}/revisions"),Some(json!({"base_revision":0,"cases":[{"id":"case","messages":[{"role":"user","content":"What is 2+2?"}],"expected":"4","source":{},"request":{"messages":[{"role":"user","content":"What is 2+2?"}],"parameters":{"temperature":0.2}}}]}))).await;
    let created = api(&server,"/lens/optimizations",Some(json!({"mode":"offline","agent":"calculator","baseline":"baseline","models":["candidate"],"teamId":"","traffic":10,"judge":"llm","judgeModel":"judge","rubric":"Is the answer correct?","baselineSampling":100,"candidateSampling":100,"datasetId":dataset_id,"datasetName":"Arithmetic","revision":1,"scope":"final","tolerance":0.1}))).await;
    let saved = api(&server, &format!("/lens/datasets/{dataset_id}"), None).await;
    assert_eq!(
        saved["cases"][0]["request"]["parameters"]["temperature"],
        0.2
    );
    let stopped = api(
        &server,
        "/lens/optimizations",
        Some(created["config"].clone()),
    )
    .await;
    let stopped_id = stopped["id"].as_str().unwrap();
    let stopped = api(
        &server,
        &format!("/lens/optimizations/{stopped_id}/status"),
        Some(json!({"status":"completed"})),
    )
    .await;
    assert_eq!(stopped["pending"], 0);
    assert_eq!(stopped["results"].as_array().unwrap().len(), 2);
    assert!(stopped["results"][0]["error"].is_string());
    assert_eq!(created["pending"], 2);
    let id = created["id"].as_str().unwrap();
    let route = format!("/lens/optimizations/{id}");
    api(
        &server,
        &format!("{route}/status"),
        Some(json!({"status":"paused"})),
    )
    .await;
    server.optimizations.tick(0).await.unwrap();
    assert_eq!(api(&server, &route, None).await["pending"], 2);
    api(
        &server,
        &format!("{route}/status"),
        Some(json!({"status":"running"})),
    )
    .await;
    server.optimizations.tick(0).await.unwrap();
    drop(server);
    let restarted = database.serve_registry(true, models()).await;
    restarted.optimizations.tick(0).await.unwrap();
    restarted.optimizations.tick(0).await.unwrap();
    let finished = api(&restarted, &route, None).await;
    assert_eq!(finished["status"], "completed");
    assert_eq!(finished["pending"], 0);
    let results = finished["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(
        results
            .iter()
            .map(|result| result["score"].as_f64().unwrap())
            .sum::<f64>(),
        20.0
    );
    assert_eq!(
        results
            .iter()
            .map(|result| result["costUsd"].as_f64().unwrap())
            .sum::<f64>(),
        0.003
    );
    assert_eq!(
        gateway
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        4
    );
}

fn identity(role: &str, team: &str, models: Vec<&str>) -> String {
    let now = chrono::Utc::now().timestamp();
    jsonwebtoken::encode(&jsonwebtoken::Header::default(), &json!({
        "iss":"litellm","aud":"litellm-lens","sub":"fixture-user","iat":now,"exp":now+30,
        "identity":{"user_role":role,"user_id":"fixture-user","team_id":team,"org_id":null,"token":null,"models":models,"log_team_ids":[team]}
    }), &jsonwebtoken::EncodingKey::from_secret(datasets::support::GATEWAY.as_bytes())).unwrap()
}

#[rstest]
#[tokio::test]
async fn live_assignment_rechecks_permissions_is_team_scoped_and_caps_admission(
    #[future(awt)] database: Database,
) {
    let gateway = MockServer::start().await;
    Mock::given(method("GET")).and(path("/model_group/info")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"model_group":"baseline","mode":"chat"},{"model_group":"candidate","mode":"chat"}]}))).mount(&gateway).await;
    let models = Models::new(
        vec![],
        vec![],
        None,
        Some(GatewayConfig::new(&gateway.uri(), "fixture-only".into()).unwrap()),
    )
    .unwrap();
    let server = database.serve_registry(true, models).await;
    let run=api(&server,"/lens/optimizations",Some(json!({"mode":"live","agent":"support","baseline":"baseline","models":["candidate"],"teamId":"team-a","traffic":100,"judge":"feedback","judgeModel":"","rubric":"","baselineSampling":100,"candidateSampling":100,"datasetId":"","datasetName":"","revision":0,"scope":"final","tolerance":0.1}))).await;
    let request = json!({"agent":"support","baseline":"baseline","traceId":"trace"});
    let call = |token: String, body: Value| {
        reqwest::Client::new()
            .post(format!("{}/lens/optimizations/assign", server.url))
            .bearer_auth(token)
            .json(&body)
            .send()
    };
    let initial = call(
        identity("internal_user", "team-a", vec!["baseline", "candidate"]),
        request.clone(),
    )
    .await
    .unwrap();
    assert_eq!(initial.status(), 200, "{}", initial.text().await.unwrap());
    let first: Value = call(
        identity("internal_user", "team-a", vec!["baseline", "candidate"]),
        request.clone(),
    )
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(first["model"], "candidate");
    let changed: Value = call(
        identity("internal_user", "team-a", vec!["baseline"]),
        request.clone(),
    )
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(changed["model"], "baseline");
    let other: Value = call(
        identity("internal_user", "team-b", vec!["baseline", "candidate"]),
        request.clone(),
    )
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(other["optimizationId"], Value::Null);
    let viewer = call(
        identity(
            "proxy_admin_viewer",
            "team-a",
            vec!["baseline", "candidate"],
        ),
        request,
    )
    .await
    .unwrap();
    assert_eq!(viewer.status(), 403);
    let key = format!("optimization-admission/{}", run["id"].as_str().unwrap());
    let previous = server.store.read(&key).await.unwrap();
    server
        .store
        .commit(vec![litellm_storage_clickhouse::state::Change {
            previous,
            value: json!(5000),
        }])
        .await
        .unwrap();
    let full: Value = call(
        identity("internal_user", "team-a", vec!["baseline", "candidate"]),
        json!({"agent":"support","baseline":"baseline","traceId":"another"}),
    )
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(full["model"], "baseline");
    assert_eq!(
        server
            .store
            .keys(
                &format!("optimization-assignment/{}/", run["id"].as_str().unwrap()),
                "",
                10
            )
            .await
            .unwrap()
            .len(),
        1
    );
}

#[rstest]
#[case::anonymous(None, 401)]
#[case::viewer(Some("proxy_admin_viewer"), 403)]
#[case::team(Some("internal_user"), 403)]
#[tokio::test]
async fn optimization_mutations_require_admin(
    #[future(awt)] database: Database,
    #[case] role: Option<&str>,
    #[case] status: u16,
) {
    let server = database.serve(true).await;
    let request = reqwest::Client::new()
        .post(format!(
            "{}/lens/optimizations/00000000-0000-0000-0000-000000000001/status",
            server.url
        ))
        .json(&json!({"status":"paused"}));
    let response = match role {
        Some(role) => request.bearer_auth(identity(role, "team-a", vec![])),
        None => request,
    }
    .send()
    .await
    .unwrap();
    assert_eq!(response.status().as_u16(), status);
}
