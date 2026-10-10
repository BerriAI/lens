use lens_auth::{Authentication, Settings};
use litellm_lens::{SourceReader, State, Storage, config::http_client};
use litellm_storage_clickhouse::{
    Connection, execute_statement, sessions::Sessions, state::ClickHouseState,
};
use litellm_traces_clickhouse::Config;
use rstest::fixture;
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use testcontainers_modules::{
    clickhouse::ClickHouse,
    testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner},
};

pub const ADMIN: &str = "feedback-test-admin-token-with-32-characters";
pub const SECRET: &str = "feedback-test-gateway-secret-with-32-characters";

pub struct Database {
    _container: Option<ContainerAsync<ClickHouse>>,
    pub state: Arc<State>,
    pub sessions: ClickHouseState,
    pub investigations: litellm_storage_clickhouse::investigations::Investigations,
}
impl Drop for Database {
    fn drop(&mut self) {
        if self._container.is_some() {
            return;
        }
        let config = self.state.storage.config.storage();
        let connection = config.writer().clone();
        let database = config.database().to_owned();
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    execute_statement(
                        &http_client().unwrap(),
                        &connection,
                        &format!("DROP DATABASE `{database}` SYNC"),
                        Duration::from_secs(30),
                    )
                    .await
                    .unwrap();
                })
        })
        .join()
        .unwrap();
    }
}

#[fixture]
pub async fn database() -> Database {
    let (container, url) = match std::env::var("CLICKHOUSE_STATE_TEST_URL") {
        Ok(url) => (None, url),
        Err(_) => {
            let container=ClickHouse::default().with_tag("26.9.6.6@sha256:eb4870e7ca7ed70c259eebfcfbee6cf797017f6b5436c2926bbbfe3d4d28486e")
                .with_env_var("CLICKHOUSE_SKIP_USER_SETUP","1")
                .with_copy_to("/etc/clickhouse-server/config.d/lens-keeper.xml",include_bytes!("../../../storage-clickhouse/tests/state/fixtures/keeper.xml").to_vec())
                .start().await.unwrap();
            let url = format!(
                "http://{}:{}",
                container.get_host().await.unwrap(),
                container.get_host_port_ipv4(8123).await.unwrap()
            );
            (Some(container), url)
        }
    };
    let name = format!("lens_activity_test_{}", uuid::Uuid::new_v4().simple());
    let config = Config::new(name.clone(), &url, 14).unwrap();
    let sessions = ClickHouseState::new(
        http_client().unwrap(),
        Connection::reader(&url, &name).unwrap(),
    );
    let state = Arc::new(State::standalone(Storage::new(
        config,
        http_client().unwrap(),
        SECRET.into(),
    )));
    state.storage.ensure_schema().await.unwrap();
    sessions
        .initialize(&format!("/feedback-tests/{name}"))
        .await
        .unwrap();
    state.schema_ready.store(true, Ordering::Release);
    let investigations =
        litellm_storage_clickhouse::investigations::Investigations(sessions.clone());
    investigations.initialize().await.unwrap();
    Database {
        _container: container,
        state,
        sessions,
        investigations,
    }
}

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

impl Database {
    pub async fn serve(&self, enabled: bool) -> Server {
        self.serve_models(enabled, Vec::new()).await
    }

    pub async fn serve_models(
        &self,
        enabled: bool,
        models: Vec<lens_contract::activity::AnalysisModelInfo>,
    ) -> Server {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url: url::Url = format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let authentication = Arc::new(Authentication {
            settings: Settings::new(ADMIN, Some(SECRET.into()), url.as_str()).unwrap(),
            sessions: Sessions(self.sessions.clone()),
        });
        let router = lens_server::sessions::router_with_auth(authentication.clone()).merge(
            lens_server::activity::router(
                authentication.clone(),
                self.investigations.clone(),
                enabled.then(|| SourceReader(self.state.clone())),
            ),
        );
        let router = router.merge(lens_server::models::router(authentication, models));
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Server {
            url,
            client: reqwest::Client::new(),
            task,
        }
    }

    pub async fn trace(&self, id: &str, team: &str, at: i64) {
        self.trace_with_agent(id, team, at, "").await;
    }

    pub async fn trace_with_agent(&self, id: &str, team: &str, at: i64, agent: &str) {
        let row: BTreeMap<String, serde_json::Value> = BTreeMap::from([
            (
                "Timestamp".into(),
                json!(
                    chrono::DateTime::from_timestamp_nanos(at)
                        .to_rfc3339_opts(chrono::SecondsFormat::Nanos, true)
                ),
            ),
            ("TraceId".into(), json!(id)),
            ("SpanId".into(), json!("0101010101010101")),
            ("SpanName".into(), json!("feedback fixture")),
            ("ServiceName".into(), json!("fixture")),
            ("AgentName".into(), json!(agent)),
            ("TeamId".into(), json!(team)),
            ("ApiKeyHash".into(), json!("feedback-key")),
            ("Duration".into(), json!(1_000_000_000_u64)),
            ("EngineReceivedMs".into(), json!(at / 1_000_000 + 1000)),
        ]);
        let storage = &self.state.storage;
        execute_statement(
            &storage.client,
            storage.config.storage().writer(),
            &format!(
                "INSERT INTO `{}`.otel_traces FORMAT JSONEachRow\n{}",
                storage.config.storage().database(),
                serde_json::to_string(&row).unwrap()
            ),
            Duration::from_secs(30),
        )
        .await
        .unwrap();
    }
}

pub fn delegated(identity: lens_contract::auth::Identity) -> String {
    let now = chrono::Utc::now().timestamp();
    let subject = identity
        .user_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .or(identity.token.as_deref())
        .unwrap_or("caller");
    jsonwebtoken::encode(&jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),&json!({"iss":"litellm","aud":"litellm-lens","sub":subject,"iat":now,"exp":now+60,"identity":identity}),&jsonwebtoken::EncodingKey::from_secret(SECRET.as_bytes())).unwrap()
}
