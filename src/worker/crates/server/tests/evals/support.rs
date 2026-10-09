use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, Response},
};
use chrono::Utc;
use jsonwebtoken::{EncodingKey, Header, encode};
use lens_auth::{Authentication, Settings};
use lens_contract::eval::EvalRun;
use litellm_http::Client;
use litellm_storage_clickhouse::{
    Connection,
    evals::EvalStore,
    execute_statement,
    sessions::Sessions,
    state::{Change, ClickHouseState},
};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rstest::fixture;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use testcontainers_modules::{
    clickhouse::ClickHouse,
    testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner},
};
use tower::ServiceExt;

const SECRET: &str = "eval-route-test-signing-secret-32-characters";
pub const ADMIN: &str = "eval-route-test-admin-token-32-characters";

#[fixture]
pub fn guarded_app() -> Router {
    let state = ClickHouseState::new(
        Client::no_redirect_for_test(),
        Connection::reader("http://127.0.0.1:9", "unreachable").unwrap(),
    );
    let auth = Arc::new(Authentication {
        settings: Settings::new(
            "eval-route-test-admin-token-32-characters",
            Some(SECRET.to_owned()),
            "http://lens.test",
        )
        .unwrap(),
        sessions: Sessions(state.clone()),
    });
    lens_server::evals::router(auth, state, "http://lens.test".into())
}

pub struct EvalFixture {
    pub app: Router,
    pub store: EvalStore,
    _container: Option<ContainerAsync<ClickHouse>>,
    connection: Connection,
    name: String,
}

impl EvalFixture {
    pub fn combined_app(&self) -> Router {
        let state = self.store.state().clone();
        let auth = Arc::new(Authentication {
            settings: Settings::new(ADMIN, Some(SECRET.into()), "http://lens.test").unwrap(),
            sessions: Sessions(state.clone()),
        });
        lens_server::datasets::router_with_evals(
            auth.clone(),
            litellm_storage_clickhouse::datasets::Datasets(state.clone()),
            NoDatasetSource,
            lens_server::datasets::DatasetConfig::default(),
            state.clone(),
            None,
        )
        .merge(lens_server::evals::router_without_cases(
            auth,
            state,
            "http://lens.test".into(),
            None,
        ))
    }
}

struct NoDatasetSource;

impl lens_datasets::DatasetReader for NoDatasetSource {
    async fn trace(
        &self,
        _: &str,
        _: &str,
    ) -> Result<Option<litellm_traces::Trace>, lens_datasets::ReadError> {
        Ok(None)
    }

    async fn span(
        &self,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<Option<litellm_traces::SpanDetail>, lens_datasets::ReadError> {
        Ok(None)
    }

    async fn findings(
        &self,
        _: &str,
        _: &[String],
        _: &lens_datasets::Scope,
    ) -> Result<Vec<lens_datasets::Finding>, lens_datasets::ReadError> {
        Ok(Vec::new())
    }
}

impl Drop for EvalFixture {
    fn drop(&mut self) {
        if self._container.is_some() {
            return;
        }
        let connection = self.connection.clone();
        let name = self.name.clone();
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    execute_statement(
                        &Client::no_redirect_for_test(),
                        &connection,
                        &format!("DROP DATABASE `{name}` SYNC"),
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

#[fixture]
pub async fn eval_fixture() -> EvalFixture {
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
    let name = format!("lens_eval_routes_{}", uuid::Uuid::new_v4().simple());
    let connection = Connection::reader(&url, &name).unwrap();
    execute_statement(
        &Client::no_redirect_for_test(),
        &Connection::writer(&url).unwrap(),
        &format!("CREATE DATABASE `{name}`"),
        Duration::from_secs(30),
    )
    .await
    .unwrap();
    let state = ClickHouseState::new(Client::no_redirect_for_test(), connection.clone());
    state
        .initialize(&format!("/eval-route-tests/{name}"))
        .await
        .unwrap();
    seed(&state, "dataset-latest", json!(["dataset-1"]), json!({
        "team_id":"team-a", "summary": {"id":"dataset-1","name":"regressions","revision":7,"agent_name":"agent","case_count":3}
    })).await;
    seed(&state, "dataset", json!(["dataset-1",7]), json!({
        "team_id":"team-a","id":"dataset-1","name":"regressions","revision":7,
        "cases":[
            {"id":"case-1","messages":[{"role":"user","content":"first"}],"expected":"works","source":{"lens_id":"lens-1","finding_id":"finding-1"},"meta":{"repo_url":"https://example.test/repo","secret":"private"}},
            {"id":"case-2","messages":[],"source":{}},
            {"id":"excluded","messages":[],"included":false,"source":{}}
        ]
    })).await;
    seed(&state, "lens", json!(["lens-1"]), json!({"lens":{"scope":{"team_id":"team-a"},"findings":[{"id":"finding-1","title":"Run tests","priority":"high"}]}})).await;
    let auth = Arc::new(Authentication {
        settings: Settings::new(
            "eval-route-test-admin-token-32-characters",
            Some(SECRET.to_owned()),
            "http://lens.test",
        )
        .unwrap(),
        sessions: Sessions(state.clone()),
    });
    EvalFixture {
        app: lens_server::evals::router(auth, state.clone(), "http://lens.test".into()),
        store: EvalStore::new(state),
        _container: container,
        connection,
        name,
    }
}

async fn seed(state: &ClickHouseState, namespace: &str, identity: Value, value: Value) {
    let identity = serde_json::to_string(&identity).unwrap();
    const CHARACTERS: &percent_encoding::AsciiSet = &NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'~');
    let key = format!("{namespace}/{}", utf8_percent_encode(&identity, CHARACTERS));
    let previous = state.read(&key).await.unwrap();
    state
        .commit(vec![Change { previous, value }])
        .await
        .unwrap();
}

pub fn token(team: Option<&str>) -> String {
    let now = Utc::now().timestamp();
    encode(&Header::default(), &json!({
        "iss":"litellm","aud":"litellm-lens","sub":"test-key","iat":now,"exp":now + 60,
        "identity":{"user_role":"team","user_id":null,"team_id":team,"org_id":null,"token":"test-key","models":[],"log_team_ids":[]}
    }), &EncodingKey::from_secret(SECRET.as_bytes())).unwrap()
}

pub fn request(method: &str, path: &str, team: &str, body: Option<Value>) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {}", token(Some(team))))
        .header("X-Lens-Contract", "1")
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
        .unwrap()
}

pub fn create_payload() -> Value {
    json!({"eval":"regressions","agent":"agent","dataset_id":"dataset-1","revision":7,"version":"abc123","branch":"main","trials":3,"timeout_per_trial_ms":100,"scorers":[{"kind":"task_completed"}]})
}

pub async fn create(fixture: &EvalFixture) -> EvalRun {
    let response = fixture
        .app
        .clone()
        .oneshot(request(
            "POST",
            "/lens/evals/runs",
            "team-a",
            Some(create_payload()),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    body(response).await
}

pub async fn body<T: DeserializeOwned>(response: Response<Body>) -> T {
    serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
}
