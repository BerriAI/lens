use std::time::Duration;

use chrono::{DateTime, Utc};
use lens_auth::{IngestionError, ingestion::IngestionRepository};
use lens_contract::ingestion::{IngestionKey, IngestionTenant};
use litellm_http::Client;
use litellm_storage_clickhouse::{
    Connection, execute_statement,
    ingestion::IngestionKeys,
    state::{Change, ClickHouseState, Snapshot},
};
use rstest::fixture;
use serde_json::Value;
use testcontainers_modules::{
    clickhouse::ClickHouse,
    testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner},
};

pub const CATALOG: &str = "ingestion-keys/catalog";

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
    pub fn repository(&self) -> IngestionKeys {
        IngestionKeys(ClickHouseState::new(
            Client::no_redirect_for_test(),
            self.connection.clone(),
        ))
    }

    pub async fn seed(&self, value: Value) {
        self.store
            .commit(vec![Change {
                previous: Snapshot::empty(CATALOG),
                value,
            }])
            .await
            .unwrap();
    }

    pub async fn restart(&self) -> IngestionKeys {
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
                if store.heads(&[CATALOG]).await.is_ok() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
        IngestionKeys(store)
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
    let name = format!("lens_rust_ingestion_{}", uuid::Uuid::new_v4().simple());
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
        .initialize(&format!("/ingestion-tests/{name}"))
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

pub fn key(id: &str) -> IngestionKey {
    IngestionKey {
        id: id.into(),
        name: format!("Agent {id}"),
        tenant: IngestionTenant {
            user_id: "owner".into(),
            team_id: "team".into(),
            org_id: String::new(),
            api_key_hash: format!("{id:0>64}"),
        },
        created_at: now(),
        expires_at: None,
    }
}

pub fn catalog(count: usize) -> Value {
    serde_json::to_value(
        (0..count)
            .map(|index| key(&format!("{index:08}")))
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

pub async fn concurrent_inserts(
    database: &Database,
    identical: bool,
) -> Vec<Result<(), IngestionError>> {
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(16));
    let mut tasks = tokio::task::JoinSet::new();
    for index in 0..16 {
        let repository = database.repository();
        let barrier = barrier.clone();
        tasks.spawn(async move {
            let key = key(&if identical {
                "one-id".into()
            } else {
                format!("key-{index:02}")
            });
            barrier.wait().await;
            repository.insert(&key).await
        });
    }
    let mut results = Vec::new();
    while let Some(result) = tasks.join_next().await {
        results.push(result.unwrap());
    }
    results
}

pub async fn concurrent_insert_and_revoke(database: &Database) {
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(16));
    let mut tasks = tokio::task::JoinSet::new();
    for index in 0..16 {
        let repository = database.repository();
        let barrier = barrier.clone();
        tasks.spawn(async move {
            barrier.wait().await;
            if index % 2 == 0 {
                repository.insert(&key(&format!("new-{index:02}"))).await
            } else {
                repository.revoke(&format!("{index:08}")).await
            }
        });
    }
    while let Some(result) = tasks.join_next().await {
        result.unwrap().unwrap();
    }
}

pub fn survivors() -> Vec<IngestionKey> {
    (0..16)
        .step_by(2)
        .map(|index| key(&format!("{index:08}")))
        .chain(
            (0..16)
                .step_by(2)
                .map(|index| key(&format!("new-{index:02}"))),
        )
        .collect()
}
