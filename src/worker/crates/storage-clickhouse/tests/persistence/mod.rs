use std::time::Duration;

use chrono::{DateTime, Utc};
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
    container: Option<ContainerAsync<ClickHouse>>,
    connection: Connection,
    name: String,
    pub store: ClickHouseState,
}

impl Drop for Database {
    fn drop(&mut self) {
        if self.container.is_some() {
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
                        &format!("DROP DATABASE IF EXISTS `{name}` SYNC"),
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

impl Database {
    pub async fn sql(&self, sql: &str) {
        execute_statement(
            &Client::no_redirect_for_test(),
            &self.connection,
            sql,
            Duration::from_secs(30),
        )
        .await
        .unwrap();
    }

    pub fn independent(&self) -> ClickHouseState {
        ClickHouseState::new(Client::no_redirect_for_test(), self.connection.clone())
    }

    pub async fn seed(&self, key: &str, value: Value) {
        self.store
            .commit(vec![Change {
                previous: Snapshot::empty(key),
                value,
            }])
            .await
            .unwrap();
    }

    pub async fn restart(&self) -> ClickHouseState {
        let container = self.container.as_ref().unwrap();
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
        .unwrap();
        store
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
                .with_copy_to("/etc/clickhouse-server/config.d/lens-keeper.xml", include_bytes!("../state/fixtures/keeper.xml").to_vec())
                .start().await.unwrap();
            let url = format!(
                "http://{}:{}",
                container.get_host().await.unwrap(),
                container.get_host_port_ipv4(8123).await.unwrap()
            );
            (Some(container), url)
        }
    };
    let name = format!("lens_rust_persistence_{}", uuid::Uuid::new_v4().simple());
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
        .initialize(&format!("/persistence-tests/{name}"))
        .await
        .unwrap();
    Database {
        container,
        connection,
        name,
        store,
    }
}

#[fixture]
pub fn now() -> DateTime<Utc> {
    "2026-03-01T12:00:00.123456Z".parse().unwrap()
}
