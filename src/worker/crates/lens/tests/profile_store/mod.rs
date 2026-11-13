use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use litellm_lens::{State, Storage, config::http_client, task_profiles::ProfilesApi};
use litellm_storage_clickhouse::{Connection, execute_statement};
use litellm_traces_clickhouse::Config;
use testcontainers_modules::{
    clickhouse::ClickHouse,
    testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner},
};

const SECRET: &str = "isolated-test-internal-secret-32-bytes";

/// One test-owned database on the shared ClickHouse that CI exports as `CLICKHOUSE_STATE_TEST_URL`,
/// or on a private container when it is unset. Dropped with the test.
pub struct Isolated {
    _container: Option<ContainerAsync<ClickHouse>>,
    url: String,
    database: String,
    pub client: litellm_http::Client,
    pub config: Config,
    pub profiles: ProfilesApi,
}

impl Drop for Isolated {
    fn drop(&mut self) {
        if self._container.is_some() {
            return;
        }
        let url = self.url.clone();
        let database = self.database.clone();
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime")
                .block_on(async {
                    let dropped = execute_statement(
                        &http_client().expect("client"),
                        &Connection::writer(&url).expect("writer"),
                        &format!("DROP DATABASE IF EXISTS `{database}` SYNC"),
                        Duration::from_secs(30),
                    )
                    .await;
                    if let Err(error) = dropped {
                        eprintln!("test database cleanup failed: {database}: {error}");
                    }
                });
        })
        .join()
        .expect("cleanup");
    }
}

pub async fn isolated() -> Isolated {
    let (container, url) = match std::env::var("CLICKHOUSE_STATE_TEST_URL") {
        Ok(url) => (None, url),
        Err(_) => {
            let container = ClickHouse::default()
                .with_tag("26.9.6.6@sha256:eb4870e7ca7ed70c259eebfcfbee6cf797017f6b5436c2926bbbfe3d4d28486e")
                .with_env_var("CLICKHOUSE_SKIP_USER_SETUP", "1")
                .start()
                .await
                .expect("ClickHouse");
            let url = format!(
                "http://{}:{}",
                container.get_host().await.expect("host"),
                container.get_host_port_ipv4(8123).await.expect("port")
            );
            (Some(container), url)
        }
    };
    let client = http_client().expect("client");
    let database = format!("lens_profiles_{}", uuid::Uuid::new_v4().simple());
    execute_statement(
        &client,
        &Connection::writer(&url).expect("writer"),
        &format!("CREATE DATABASE `{database}`"),
        Duration::from_secs(30),
    )
    .await
    .expect("database");
    let config = Config::new(database.clone(), &url, 14).expect("config");
    let storage = Storage::new(config.clone(), client.clone(), SECRET.into());
    storage.ensure_schema().await.expect("schema");
    let state = Arc::new(State::connected(storage, SECRET.into()));
    state.schema_ready.store(true, Ordering::Release);
    Isolated {
        _container: container,
        url,
        database,
        client,
        config,
        profiles: ProfilesApi::new(state),
    }
}
