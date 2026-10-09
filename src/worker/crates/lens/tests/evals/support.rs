use lens_auth::{Authentication, Settings};
use lens_contract::datasets::{CaseSource, Dataset, DatasetCase, DatasetMessage, DatasetRole};
use lens_datasets::DatasetRepository;
use litellm_storage_clickhouse::{datasets::Datasets, evals::Evals, sessions::Sessions};
use serde_json::{Value, json};
use std::sync::Arc;

use crate::activity::support::{ADMIN, Database, SECRET};

pub struct Server {
    pub url: url::Url,
    pub client: reqwest::Client,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    pub fn request(&self, method: &str, path: &str) -> reqwest::RequestBuilder {
        self.client
            .request(method.parse().unwrap(), self.url.join(path).unwrap())
            .bearer_auth(ADMIN)
            .header("x-lens-contract", "1")
    }
}

pub async fn serve(database: &Database) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url: url::Url = format!("http://{}", listener.local_addr().unwrap())
        .parse()
        .unwrap();
    let authentication = Arc::new(Authentication {
        settings: Settings::new(ADMIN, Some(SECRET.into()), url.as_str()).unwrap(),
        sessions: Sessions(database.sessions.clone()),
    });
    let repository = Datasets(database.sessions.clone());
    repository
        .insert(&dataset(), chrono::Utc::now())
        .await
        .unwrap();
    let router = lens_server::evals::router(
        authentication,
        repository,
        Evals(database.sessions.clone()),
        lens_server::evals::EvalConfig {
            public_url: url.clone(),
        },
    );
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Server {
        url,
        client: reqwest::Client::new(),
        task,
    }
}

pub fn dataset() -> Dataset {
    Dataset {
        id: "eval-dataset".into(),
        name: "Eval dataset".into(),
        agent_name: "test-agent".into(),
        team_id: "alpha".into(),
        created_at: chrono::Utc::now(),
        revision: 1,
        created_by: "admin".into(),
        cases: vec![DatasetCase {
            id: "case-one".into(),
            messages: vec![DatasetMessage {
                role: DatasetRole::User,
                content: "Complete the task".into(),
                name: String::new(),
                tool_calls: Vec::new(),
            }],
            reply: String::new(),
            tool_calls: Vec::new(),
            expected: "Task completed".into(),
            included: true,
            source: CaseSource::default(),
            agent_version: String::new(),
        }],
    }
}

pub fn spec() -> Value {
    json!({"eval":"regressions","agent":"test-agent","dataset_id":"eval-dataset","revision":1,"version":"build-one","branch":"main","scorers":[{"kind":"task_completed"}],"timeout_per_trial_ms":60000})
}

pub async fn create(server: &Server, key: &str, spec: Value) -> lens_contract::eval::EvalRun {
    let response = server
        .request("POST", "/lens/evals/runs")
        .header("idempotency-key", key)
        .json(&spec)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert_eq!(status, 201, "{text}");
    serde_json::from_str(&text).unwrap()
}

pub async fn trace(
    database: &Database,
    trace_id: &str,
    version: &str,
    status: &str,
    parent: &str,
    age_seconds: i64,
) {
    let now = chrono::Utc::now() - chrono::TimeDelta::seconds(age_seconds);
    let storage = &database.state.storage;
    litellm_storage_clickhouse::execute_statement(&storage.client,storage.config.storage().writer(),&format!(
        "INSERT INTO `{}`.otel_traces FORMAT JSONEachRow\n{}",storage.config.storage().database(),
        json!({"Timestamp":now.to_rfc3339_opts(chrono::SecondsFormat::Nanos,true),"TraceId":trace_id,"SpanId":"root","ParentSpanId":parent,"SpanName":"Agent task","TeamId":"alpha","ApiKeyHash":"eval-key","Duration":1000000,"EngineReceivedMs":now.timestamp_millis(),"StatusCode":status,"Input":"Complete the task","Output":"Task completed with evidence","SpanAttributes":{"agent.version":version,"session.id":trace_id}})
    ),std::time::Duration::from_secs(30)).await.unwrap();
}

pub fn evaluator(database: &Database) -> litellm_lens::evaluations::Evaluations<Evals> {
    litellm_lens::evaluations::Evaluations::new(
        Evals(database.sessions.clone()),
        database.state.clone(),
        Arc::new(
            lens_analysis::AnalysisModels::new(
                lens_analysis::bundled_catalog().unwrap(),
                Vec::new(),
                Default::default(),
            )
            .unwrap(),
        ),
    )
}

pub async fn submit(server: &Server, id: &str, result: Value) {
    let response = server
        .request("PUT", &format!("/lens/evals/runs/{id}/results/case-one/0"))
        .json(&result)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 204, "{}", response.text().await.unwrap());
}

pub async fn finish(server: &Server, id: &str) {
    let response = server
        .request("POST", &format!("/lens/evals/runs/{id}/finish"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 202);
    assert_eq!(response.json::<Value>().await.unwrap()["status"], "scoring");
}

pub async fn get(server: &Server, id: &str) -> lens_contract::eval::EvalRun {
    let response = server
        .request("GET", &format!("/lens/evals/runs/{id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    response.json().await.unwrap()
}

pub async fn priced_trace(database: &Database, id: &str) {
    trace(database, id, "build-one", "ok", "", 2).await;
    let now = chrono::Utc::now() - chrono::TimeDelta::seconds(1);
    let at = now.to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);
    let storage = &database.state.storage;
    litellm_storage_clickhouse::execute_statement(&storage.client, storage.config.storage().writer(), &format!(
        "INSERT INTO `{}`.otel_traces FORMAT JSONEachRow\n{}", storage.config.storage().database(),
        json!({"Timestamp":at,"TraceId":id,"SpanId":"llm","ParentSpanId":"root","SpanName":"model call","ObservationType":"llm","TeamId":"alpha","ApiKeyHash":"eval-key","Duration":1000000,"EngineReceivedMs":now.timestamp_millis(),"StatusCode":"ok","SpanAttributes":{"agent.version":"build-one"},"LiteLLMRequestId":"priced-request","CallKeys":[litellm_traces::CallKey::LiteLlmRequest("priced-request".into())]})
    ),std::time::Duration::from_secs(30)).await.unwrap();
    litellm_storage_clickhouse::execute_statement(&storage.client, storage.config.storage().writer(), &format!(
        "INSERT INTO `{}`.spend_logs FORMAT JSONEachRow\n{}", storage.config.storage().database(),
        json!({"request_id":"priced-request","trace_id":id,"span_id":"llm","team_id":"alpha","api_key":"eval-key","model":"fixture","start_time":at,"end_time":at,"spend":0.37})
    ),std::time::Duration::from_secs(30)).await.unwrap();
}
