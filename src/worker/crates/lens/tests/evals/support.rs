use std::{collections::BTreeMap, sync::Arc, time::Duration};

use axum::Router;
use chrono::Utc;
use lens_contract::eval::{CreateEvalRun, EvalRun, RunStatus, Summary};
use lens_evals_sdk::devserver;
use lens_server::eval_closer::EvalCloser;
use litellm_lens::{eval_judge::GatewayJudge, eval_runtime::TraceReader, eval_scoring::EvalScorer};
use litellm_storage_clickhouse::{
    Connection,
    evals::EvalStore,
    execute_statement,
    sessions::Sessions,
    state::{Change, ClickHouseState, Snapshot},
};
use litellm_traces_clickhouse::{InsertTable, ensure_schema, evals::EvalTraces, insert_rows};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::Method;
use rstest::fixture;
use serde_json::{Value, json};
use testcontainers_modules::{
    clickhouse::ClickHouse,
    testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner},
};

const SECRET: &str = "golden-eval-gateway-secret";
const TEAM: &str = "golden-team";
const CASES: usize = 36;
const TRIALS: u32 = 3;

pub struct Server {
    client: reqwest::Client,
    url: String,
    key: Option<String>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(router: impl FnOnce(&str) -> Router, key: Option<String>) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = router(&url);
    Server {
        client: reqwest::Client::new(),
        url,
        key,
        task: tokio::spawn(async move { axum::serve(listener, app).await.unwrap() }),
    }
}

pub struct Fixture {
    pub actual: Server,
    pub dev: Server,
    pub store: EvalStore,
    traces: EvalTraces,
    http: litellm_http::Client,
    writer: Connection,
    database: String,
    container: Option<ContainerAsync<ClickHouse>>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if self.container.is_some() {
            return;
        }
        let connection = self.writer.clone();
        let database = self.database.clone();
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    execute_statement(
                        &litellm_http::Client::no_redirect_for_test(),
                        &connection,
                        &format!("DROP DATABASE `{database}` SYNC"),
                        Duration::from_secs(30),
                    )
                    .await
                    .unwrap();
                });
        })
        .join()
        .unwrap();
    }
}

fn key(namespace: &str, identity: Value) -> String {
    const CHARACTERS: &percent_encoding::AsciiSet = &NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'~');
    format!(
        "{namespace}/{}",
        utf8_percent_encode(&identity.to_string(), CHARACTERS)
    )
}

fn gateway_token() -> String {
    let now = Utc::now().timestamp();
    jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &json!({
            "iss":"litellm","aud":"litellm-lens","sub":"golden-key","iat":now,"exp":now+60,
            "identity":{"user_role":"team","team_id":TEAM,"token":"golden-key"}
        }),
        &jsonwebtoken::EncodingKey::from_secret(SECRET.as_bytes()),
    )
    .unwrap()
}

#[fixture]
pub async fn fixture() -> Fixture {
    let (container, url) = match std::env::var("CLICKHOUSE_STATE_TEST_URL") {
        Ok(url) => (None, url),
        Err(_) => {
            let container = ClickHouse::default()
                .with_tag("26.9.6.6@sha256:eb4870e7ca7ed70c259eebfcfbee6cf797017f6b5436c2926bbbfe3d4d28486e")
                .with_env_var("CLICKHOUSE_SKIP_USER_SETUP", "1")
                .with_copy_to("/etc/clickhouse-server/config.d/lens-keeper.xml", include_bytes!("../../../storage-clickhouse/tests/state/fixtures/keeper.xml").to_vec())
                .start().await.unwrap();
            let url = format!(
                "http://{}:{}",
                container.get_host().await.unwrap(),
                container.get_host_port_ipv4(8123).await.unwrap()
            );
            (Some(container), url)
        }
    };
    let database = format!("lens_eval_golden_{}", uuid::Uuid::new_v4().simple());
    let http = litellm_http::Client::no_redirect_for_test();
    let writer = Connection::writer(&url).unwrap();
    ensure_schema(&http, &writer, &database, 7).await.unwrap();
    let reader = Connection::reader(&url, &database).unwrap();
    let state = ClickHouseState::new(http.clone(), reader.clone());
    state
        .initialize(&format!("/eval-golden/{database}"))
        .await
        .unwrap();
    let dataset = devserver::sample_cases(CASES);
    let cases: Vec<_> = dataset
        .cases
        .iter()
        .enumerate()
        .map(|(index, case)| {
            let mut value = serde_json::to_value(case).unwrap();
            value["source"] = json!({"lens_id":"findings","finding_id":index.to_string()});
            value
        })
        .collect();
    let changes = [
        (
            key("dataset-latest", json!(["demo"])),
            json!({"team_id":TEAM,"summary":{"id":"demo","name":"demo","revision":1}}),
        ),
        (
            key("dataset", json!(["demo", 1])),
            json!({"id":"demo","name":"demo","team_id":TEAM,"revision":1,"cases":cases}),
        ),
        (
            key("lens", json!(["findings"])),
            json!({"lens":{"scope":{"team_id":TEAM},"findings":dataset.cases.iter().enumerate().map(|(index, case)| json!({"id":index.to_string(),"title":case.id,"priority":case.meta["priority"]})).collect::<Vec<_>>()}}),
        ),
    ];
    state
        .commit(
            changes
                .into_iter()
                .map(|(key, value)| Change {
                    previous: Snapshot::empty(key),
                    value,
                })
                .collect(),
        )
        .await
        .unwrap();
    let actual = serve(
        |url| {
            lens_server::evals::router(
                Arc::new(lens_auth::Authentication {
                    settings: lens_auth::Settings::new(
                        "golden-admin-token-at-least-32-characters",
                        Some(SECRET.into()),
                        url,
                    )
                    .unwrap(),
                    sessions: Sessions(state.clone()),
                }),
                state.clone(),
                url.into(),
            )
        },
        None,
    )
    .await;
    let dev = serve(
        |_| devserver::router(dataset, "lens-dev".into()),
        Some("lens-dev".into()),
    )
    .await;
    Fixture {
        actual,
        dev,
        store: EvalStore::new(state),
        traces: EvalTraces::new(http.clone(), reader),
        http,
        writer,
        database,
        container,
    }
}

async fn call(
    server: &Server,
    method: Method,
    path: &str,
    body: Option<Value>,
    key: Option<&str>,
    status: u16,
) -> Option<Value> {
    let mut request = server
        .client
        .request(method, format!("{}{path}", server.url))
        .bearer_auth(server.key.clone().unwrap_or_else(gateway_token))
        .header("X-Lens-Contract", "1");
    if let Some(body) = body {
        request = request.json(&body);
    }
    if let Some(key) = key {
        request = request.header("Idempotency-Key", key);
    }
    let response = request.send().await.unwrap();
    let code = response.status().as_u16();
    let bytes = response.bytes().await.unwrap();
    assert_eq!(code, status, "{path}: {}", String::from_utf8_lossy(&bytes));
    if status == 204 {
        None
    } else {
        Some(serde_json::from_slice(&bytes).unwrap())
    }
}

impl Fixture {
    async fn seed_traces(&self, version: &str) {
        let timestamp_ms = Utc::now().timestamp_millis() - 1000;
        let trials = || {
            (0..CASES).flat_map(|case| {
                (0..TRIALS).map(move |trial| format!("session-{version}-{case}-{trial}"))
            })
        };
        let spans = trials().map(|id| serde_json::from_value::<BTreeMap<String, Value>>(json!({
            "Timestamp":timestamp_ms*1_000_000,"Duration":1_000_000,"TraceId":id,"SpanId":"root","ParentSpanId":"",
            "SpanName":"task completed","StatusCode":"STATUS_CODE_OK","TeamId":TEAM,"ApiKeyHash":"golden-key",
            "ResourceAttributes":{"agent.name":"demo","agent.version":version,"deployment.environment":"lens-eval"},
            "SpanAttributes":{"session.id":id},"Input":"task","Output":"done"
        })).unwrap()).collect();
        insert_rows(
            &self.http,
            &self.writer,
            &self.database,
            InsertTable::OtelTraces,
            spans,
        )
        .await
        .unwrap();
        let spend = trials()
            .map(|id| {
                serde_json::from_value::<BTreeMap<String, Value>>(json!({
            "request_id":id,"team_id":TEAM,"api_key":"golden-key","trace_id":id,"span_id":"root",
            "spend":0.125,"start_time":timestamp_ms,"end_time":timestamp_ms+1
        })).unwrap()
            })
            .collect();
        insert_rows(
            &self.http,
            &self.writer,
            &self.database,
            InsertTable::SpendLogs,
            spend,
        )
        .await
        .unwrap();
    }

    pub async fn production_run(&self, version: &str, branch: &str, broken: bool) -> EvalRun {
        self.seed_traces(version).await;
        let run = submit(&self.actual, version, branch, broken, false).await;
        let pending = self.store.get(TEAM, &run.id).await.unwrap();
        assert_eq!(pending.trials.len(), CASES * TRIALS as usize);
        EvalCloser::new(
            self.store.clone(),
            TraceReader::new(self.traces.clone()),
            EvalScorer::new(GatewayJudge::unconfigured()),
        )
        .advance(&pending, Utc::now())
        .await
        .unwrap();
        read(&self.actual, &run.id).await
    }

    pub async fn dev_run(&self, version: &str, branch: &str, broken: bool) -> EvalRun {
        let run = submit(&self.dev, version, branch, broken, true).await;
        read(&self.dev, &run.id).await
    }
}

async fn submit(
    server: &Server,
    version: &str,
    branch: &str,
    broken: bool,
    explicit_cost: bool,
) -> EvalRun {
    let mut request: CreateEvalRun = serde_json::from_str(include_str!(
        "../../../contract/fixtures/lens_eval/create_run.json"
    ))
    .unwrap();
    request.version = version.into();
    request.branch = branch.into();
    let payload = serde_json::to_value(request).unwrap();
    let created = call(
        server,
        Method::POST,
        "/lens/evals/runs",
        Some(payload.clone()),
        Some(version),
        201,
    )
    .await
    .unwrap();
    let retried = call(
        server,
        Method::POST,
        "/lens/evals/runs",
        Some(payload),
        Some(version),
        201,
    )
    .await
    .unwrap();
    assert_eq!(created, retried);
    let run: EvalRun = serde_json::from_value(created).unwrap();
    for case in (0..CASES).rev() {
        for trial in (0..TRIALS).rev() {
            let result = if broken && case == 0 {
                json!({"error":{"type":"ValueError","message":"agent failed"}})
            } else {
                let prefix = if explicit_cost { "pass" } else { "session" };
                json!({"trace":{"attribute":"session.id","value":format!("{prefix}-{version}-{case}-{trial}")},"cost_usd":explicit_cost.then_some(0.125)})
            };
            let repeats = if case == CASES - 1 && trial == TRIALS - 1 {
                3
            } else {
                1
            };
            for _ in 0..repeats {
                call(
                    server,
                    Method::PUT,
                    &format!("/lens/evals/runs/{}/results/case-{case}/{trial}", run.id),
                    Some(result.clone()),
                    None,
                    204,
                )
                .await;
            }
        }
    }
    let finished: EvalRun = serde_json::from_value(
        call(
            server,
            Method::POST,
            &format!("/lens/evals/runs/{}/finish", run.id),
            None,
            None,
            202,
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(finished.status, RunStatus::Scoring);
    finished
}

async fn read(server: &Server, id: &str) -> EvalRun {
    let run: EvalRun = serde_json::from_value(
        call(
            server,
            Method::GET,
            &format!("/lens/evals/runs/{id}?wait=30"),
            None,
            None,
            200,
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(run.status, RunStatus::Done, "{}", run.failure);
    assert_eq!(run.received_trials, (CASES * TRIALS as usize) as u64);
    run
}

pub fn normalized(run: &EvalRun, baseline: Option<&EvalRun>) -> Summary {
    let mut summary = run.summary.clone().unwrap();
    assert_eq!(summary.baseline_run_id, baseline.map(|run| run.id.clone()));
    summary.baseline_run_id = baseline.map(|_| "baseline".into());
    for diff in summary
        .regressions
        .iter_mut()
        .chain(summary.fixed.iter_mut())
    {
        assert_eq!(
            diff.candidate_url,
            format!("{}&eval_case={}", run.url, diff.case_id)
        );
        assert_eq!(
            diff.baseline_url,
            format!("{}&eval_case={}", baseline.unwrap().url, diff.case_id)
        );
        diff.candidate_url = format!("candidate?case={}", diff.case_id);
        diff.baseline_url = format!("baseline?case={}", diff.case_id);
    }
    summary
}
