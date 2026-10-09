use chrono::{Duration as Interval, Utc};
use lens_auth::{Authentication, SessionId, SessionRepository, Settings};
use litellm_http::Client;
use litellm_storage_clickhouse::{
    Connection, execute_statement, sessions::Sessions, state::ClickHouseState,
};
use rstest::fixture;
use std::sync::Arc;
use std::time::Duration;
use testcontainers_modules::{
    clickhouse::ClickHouse,
    testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner},
};

pub const ADMIN: &str = "parity-admin-token-32-characters-long";
pub const SECRET: &str = "parity-gateway-secret-32-characters-long";

pub struct Database {
    _container: Option<ContainerAsync<ClickHouse>>,
    connection: Connection,
    pub store: ClickHouseState,
    pub name: String,
}

impl Drop for Database {
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
pub async fn database() -> Database {
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
    let name = format!("lens_rust_auth_{}", uuid::Uuid::new_v4().simple());
    let connection = Connection::reader(&url, &name).unwrap();
    execute_statement(
        &Client::no_redirect_for_test(),
        &Connection::writer(&url).unwrap(),
        &format!("CREATE DATABASE `{name}`"),
        Duration::from_secs(30),
    )
    .await
    .unwrap();
    let store = ClickHouseState::new(Client::no_redirect_for_test(), connection.clone());
    store
        .initialize(&format!("/auth-tests/{name}"))
        .await
        .unwrap();
    Database {
        _container: container,
        connection,
        store,
        name,
    }
}

pub struct Server {
    pub url: reqwest::Url,
    pub client: reqwest::Client,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Database {
    pub async fn serve(&self, secure: bool) -> Server {
        self.serve_router(secure, lens_server::sessions::router_with_auth)
            .await
    }

    pub async fn serve_router(
        &self,
        secure: bool,
        router: impl FnOnce(Arc<Authentication<Sessions>>) -> axum::Router,
    ) -> Server {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url: reqwest::Url = format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let origin = if secure {
            "https://lens.test"
        } else {
            url.as_str()
        };
        let auth = Authentication {
            settings: Settings::new(ADMIN, Some(SECRET.to_owned()), origin).unwrap(),
            sessions: Sessions(ClickHouseState::new(
                Client::no_redirect_for_test(),
                self.connection.clone(),
            )),
        };
        let router = router(Arc::new(auth));
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Server {
            url,
            task,
            client: reqwest::Client::new(),
        }
    }
    pub async fn seed_expired(&self) {
        Sessions(self.store.clone())
            .create(
                &SessionId::for_token("parity-expired-session"),
                Utc::now() - Interval::hours(1),
            )
            .await
            .unwrap();
    }
}

impl Server {
    pub fn endpoint(&self) -> reqwest::Url {
        self.url.join("/auth/session").unwrap()
    }
}
