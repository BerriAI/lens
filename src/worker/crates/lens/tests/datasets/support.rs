use std::{sync::Arc, time::Duration};

use lens_auth::Settings;
use litellm_lens::{
    State, Storage, api,
    auth::{Credential, Snapshot, unix_seconds},
    config::http_client,
};
use litellm_storage_clickhouse::{Connection, execute_statement, state::ClickHouseState};
use litellm_traces::Tenant;
use litellm_traces_clickhouse::Config;
use rstest::fixture;
use sha2::{Digest, Sha256};
use testcontainers_modules::{
    clickhouse::ClickHouse,
    testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner},
};

pub const ADMIN: &str = "dataset-integration-admin-token-at-least-32";
pub const INGEST: &str = "dataset-integration-tracing-key";
const SERVICE: &str = "dataset-integration-service-token-at-least-32";

pub struct Database {
    _container: Option<ContainerAsync<ClickHouse>>,
    url: String,
    pub name: String,
}

impl Drop for Database {
    fn drop(&mut self) {
        if self._container.is_some() {
            return;
        }
        let url = self.url.clone();
        let name = self.name.clone();
        std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(async {
                    execute_statement(
                        &http_client().unwrap(),
                        &Connection::writer(&url).unwrap(),
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
    Database {
        _container: container,
        url,
        name: format!("lens_dataset_e2e_{}", uuid::Uuid::new_v4().simple()),
    }
}

pub struct Server {
    pub url: String,
    pub store: ClickHouseState,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Database {
    pub async fn serve(&self, standalone: bool) -> Server {
        self.serve_with_gateway(standalone, None).await
    }

    pub async fn serve_with_gateway(
        &self,
        standalone: bool,
        gateway_secret: Option<String>,
    ) -> Server {
        let client = http_client().unwrap();
        let config = Config::new(self.name.clone(), &self.url, 14, 65_536).unwrap();
        let store = ClickHouseState::new(client.clone(), config.storage().reader().clone());
        let storage = Storage::new(config, client, SERVICE.into());
        let state = Arc::new(if standalone {
            State::standalone(storage)
        } else {
            State::new(storage, SERVICE.into())
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let application = api::initialize(
            &state,
            Settings::new(ADMIN, gateway_secret, &url).unwrap(),
            Default::default(),
            Default::default(),
            standalone,
            url.clone(),
        )
        .await
        .unwrap()
        .with_service(state.clone(), url.clone(), "standalone-test".into());
        if standalone {
            assert!(application.credentials.synchronize().await.unwrap());
        } else {
            state
                .credentials
                .replace(Snapshot {
                    issued_at: unix_seconds(),
                    keys: vec![Credential {
                        token_hash: format!("{:x}", Sha256::digest(INGEST)),
                        tenant: Tenant {
                            team_id: "dataset-team".into(),
                            user_id: "dataset-owner".into(),
                            api_key_hash: "dataset-key-hash".into(),
                            ..Tenant::default()
                        },
                        expires_at: None,
                    }],
                })
                .unwrap();
        }
        let routes = litellm_lens::router(state).merge(application.router);
        let task = tokio::spawn(async move { axum::serve(listener, routes).await.unwrap() });
        Server { url, store, task }
    }
}
