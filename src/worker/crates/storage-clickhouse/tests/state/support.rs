use std::time::Duration;

use litellm_http::Client;
use litellm_storage_clickhouse::{
    Connection, execute_statement,
    state::{Change, ClickHouseState, Snapshot},
};
use rstest::fixture;
use serde_json::Value;
use testcontainers_modules::{
    clickhouse::ClickHouse,
    testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner},
};

pub struct Database {
    _container: Option<ContainerAsync<ClickHouse>>,
    pub client: Client,
    pub connection: Connection,
    pub name: String,
    pub store: ClickHouseState,
}

impl Drop for Database {
    fn drop(&mut self) {
        if self._container.is_some() {
            return;
        }
        let url = self.connection.url().to_string();
        let name = self.name.clone();
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    let result = execute_statement(
                        &Client::no_redirect_for_test(),
                        &Connection::writer(&url).unwrap(),
                        &format!("DROP DATABASE IF EXISTS `{name}` SYNC"),
                        Duration::from_secs(30),
                    )
                    .await;
                    if let Err(error) = result {
                        eprintln!("test database cleanup failed: {name}: {error}");
                    }
                });
        })
        .join()
        .unwrap();
    }
}

impl Database {
    pub async fn restart(&self) -> ClickHouseState {
        let container = self
            ._container
            .as_ref()
            .expect("restart requires a test-owned container");
        container.stop_with_timeout(Some(0)).await.unwrap();
        container.start().await.unwrap();
        let url = format!(
            "http://{}:{}",
            container.get_host().await.unwrap(),
            container.get_host_port_ipv4(8123).await.unwrap()
        );
        let store = ClickHouseState::new(
            Client::no_redirect_for_test(),
            Connection::reader(&url, &self.name).unwrap(),
        );
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                if store.heads(&["lens"]).await.is_ok() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("ClickHouse and Keeper become ready after restart");
        store
    }

    pub fn independent(&self) -> ClickHouseState {
        ClickHouseState::new(Client::no_redirect_for_test(), self.connection.clone())
    }

    pub async fn sql(&self, sql: &str) -> String {
        let response = self
            .client
            .post(self.connection.url().clone())
            .body(sql.to_owned())
            .send()
            .await
            .unwrap();
        let status = response.status();
        let text = response.text().await.unwrap();
        assert!(status.is_success(), "{text}");
        text
    }
}

#[fixture]
pub async fn database() -> Database {
    create_database(std::env::var("CLICKHOUSE_STATE_TEST_URL").ok()).await
}

#[fixture]
pub async fn isolated_database() -> Database {
    create_database(None).await
}

async fn create_database(external_url: Option<String>) -> Database {
    let (container, url) = match external_url {
        Some(url) => (None, url),
        None => {
            let container = ClickHouse::default()
                .with_tag("26.9.6.6@sha256:eb4870e7ca7ed70c259eebfcfbee6cf797017f6b5436c2926bbbfe3d4d28486e")
                .with_env_var("CLICKHOUSE_SKIP_USER_SETUP", "1")
                .with_copy_to("/etc/clickhouse-server/config.d/lens-keeper.xml", include_bytes!("fixtures/keeper.xml").to_vec())
                .start().await.unwrap();
            let url = format!(
                "http://{}:{}",
                container.get_host().await.unwrap(),
                container.get_host_port_ipv4(8123).await.unwrap()
            );
            (Some(container), url)
        }
    };
    let name = format!("lens_rust_state_{}", uuid::Uuid::new_v4().simple());
    let client = Client::no_redirect_for_test();
    execute_statement(
        &client,
        &Connection::writer(&url).unwrap(),
        &format!("CREATE DATABASE `{name}`"),
        Duration::from_secs(30),
    )
    .await
    .unwrap();
    let connection = Connection::reader(&url, &name).unwrap();
    let store = ClickHouseState::new(client.clone(), connection.clone());
    let database = Database {
        _container: container,
        client,
        connection,
        name,
        store,
    };
    database
        .store
        .initialize(&format!("/state-tests/{}", database.name))
        .await
        .unwrap();
    database
}

pub fn change(previous: Snapshot, value: Value) -> Change {
    Change { previous, value }
}

pub fn initial(key: &str, value: Value) -> Change {
    change(Snapshot::empty(key), value)
}

pub async fn claim_concurrently(
    database: &Database,
    previous: Snapshot,
    workers: u64,
) -> Vec<(u64, Result<(), litellm_storage_clickhouse::Error>)> {
    let mut claims = tokio::task::JoinSet::new();
    for worker in 0..workers {
        let client = database.independent();
        let previous = previous.clone();
        claims.spawn(async move {
            let result = client
                .commit(vec![change(
                    previous,
                    serde_json::json!({"worker": worker}),
                )])
                .await;
            (worker, result)
        });
    }
    claims.join_all().await
}

pub async fn increment_concurrently(database: &Database, key: &str, count: u64) -> Vec<Snapshot> {
    let mut writers = tokio::task::JoinSet::new();
    for _ in 0..count {
        let client = database.independent();
        let key = key.to_owned();
        writers.spawn(async move {
            client
                .update(
                    &key,
                    |value| serde_json::json!(value.as_u64().unwrap() + 1),
                    40,
                )
                .await
                .unwrap()
        });
    }
    writers.join_all().await
}
