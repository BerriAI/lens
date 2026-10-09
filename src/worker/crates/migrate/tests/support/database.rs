use std::{collections::BTreeMap, time::Duration};

use litellm_http::Client;
use litellm_storage_clickhouse::{Connection, execute_statement, state::ClickHouseState};
use rstest::fixture;
use serde_json::Value;
use testcontainers_modules::{
    clickhouse::ClickHouse,
    testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner},
};

pub struct Database {
    _container: Option<ContainerAsync<ClickHouse>>,
    pub state: ClickHouseState,
    pub keeper: String,
    pub writer: Connection,
    pub name: String,
}

impl Drop for Database {
    fn drop(&mut self) {
        let writer = self.writer.clone();
        let name = self.name.clone();
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async move {
                    execute_statement(
                        &Client::no_redirect_for_test(),
                        &writer,
                        &format!("DROP DATABASE `{name}` SYNC"),
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

impl Database {
    pub async fn values(&self, keys: impl Iterator<Item = &String>) -> BTreeMap<String, Value> {
        let references = keys.map(String::as_str).collect::<Vec<_>>();
        self.state
            .read_many(&references)
            .await
            .unwrap()
            .into_iter()
            .map(|row| (row.head.key, row.value))
            .collect()
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
    let name = format!("lens_migration_{}", uuid::Uuid::new_v4().simple());
    let writer = Connection::writer(&url).unwrap();
    execute_statement(
        &Client::no_redirect_for_test(),
        &writer,
        &format!("CREATE DATABASE `{name}`"),
        Duration::from_secs(30),
    )
    .await
    .unwrap();
    Database {
        _container: container,
        state: ClickHouseState::new(
            Client::no_redirect_for_test(),
            Connection::reader(&url, &name).unwrap(),
        ),
        keeper: format!("/lens/{name}"),
        writer,
        name,
    }
}
