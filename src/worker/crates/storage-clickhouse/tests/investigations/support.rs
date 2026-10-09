use std::time::Duration;

use chrono::{DateTime, Utc};
use lens_contract::{
    investigations::{Lens, Public, Scope},
    worker::{Job, JobStatus},
};
use lens_investigations::{LensRepository, QueueOptions, RepositoryError, due_at, queue_job};
use litellm_http::Client;
use litellm_storage_clickhouse::{
    Connection, execute_statement,
    investigations::Investigations,
    state::{Change, ClickHouseState, Snapshot},
};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rstest::fixture;
use serde_json::{Value, json};
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
    pub async fn sql(&self, sql: &str) -> String {
        let response = Client::no_redirect_for_test()
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

    pub fn repository(&self) -> Investigations {
        Investigations(ClickHouseState::new(
            Client::no_redirect_for_test(),
            self.connection.clone(),
        ))
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

    pub async fn restart(&self) -> Investigations {
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
        let repository = Investigations(store);
        repository.initialize().await.unwrap();
        repository
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
    let name = format!("lens_rust_investigations_{}", uuid::Uuid::new_v4().simple());
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
        .initialize(&format!("/investigations-tests/{name}"))
        .await
        .unwrap();
    let database = Database {
        container,
        connection,
        name,
        store,
    };
    database.repository().initialize().await.unwrap();
    database
}

#[fixture]
pub fn now() -> DateTime<Utc> {
    "2026-03-01T12:00:00.123456Z".parse().unwrap()
}

#[fixture]
pub fn lens() -> Lens {
    serde_json::from_value(json!({
        "id": "lens", "scope": {"team_id": "alpha"},
        "settings": {"name": "Support", "model": "analysis", "checks": [{"id": "retries", "instruction": "Find retries"}]},
        "created_at": now(), "next_run_at": now(), "budget_month": "2026-03"
    })).unwrap()
}

#[fixture]
pub fn job() -> Job {
    Job {
        status: JobStatus::Completed,
        cost: 1.0,
        ..queue_job(&lens(), now(), "job", QueueOptions::default())
            .unwrap()
            .jobs
            .remove(0)
    }
}

pub fn all() -> Scope {
    Scope {
        all_teams: true,
        ..Scope::default()
    }
}
pub fn value(value: impl serde::Serialize) -> Value {
    serde_json::to_value(value).unwrap()
}
pub fn stored(lens: &Lens) -> Value {
    json!({"lens": lens, "due_at": due_at(lens)})
}
pub fn archived(lens: &Lens, job: &Job) -> Value {
    json!({"lens_id": lens.id, "parent_key": record_key("lens", &[&lens.id]), "archived_version": lens.version, "job": Public(job)})
}

pub fn record_key(namespace: &str, parts: &[&str]) -> String {
    const CHARACTERS: &percent_encoding::AsciiSet = &NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'.')
        .remove(b'_')
        .remove(b'~');
    format!(
        "{namespace}/{}",
        utf8_percent_encode(&serde_json::to_string(parts).unwrap(), CHARACTERS)
    )
}

pub fn jobs(count: usize) -> Vec<Job> {
    (0..count)
        .map(|index| Job {
            id: format!("run-{index:03}"),
            ..job()
        })
        .collect()
}

pub async fn seed_lenses(database: &Database, count: usize) -> Vec<Lens> {
    let entries: Vec<_> = (0..count)
        .map(|index| Lens {
            id: format!("lens-{index:03}"),
            ..lens()
        })
        .collect();
    let changes = entries
        .iter()
        .map(|lens| Change {
            previous: Snapshot::empty(record_key("lens", &[&lens.id])),
            value: stored(lens),
        })
        .collect();
    database.store.commit(changes).await.unwrap();
    entries
}

pub async fn concurrent_changes(
    database: &Database,
    lens: &Lens,
    create: bool,
) -> Vec<Result<Lens, RepositoryError>> {
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(16));
    let mut tasks = tokio::task::JoinSet::new();
    for index in 0..16 {
        let repository = database.repository();
        let barrier = barrier.clone();
        let lens = lens.clone();
        tasks.spawn(async move {
            let candidate = Lens {
                spent: index as f64 + 1.0,
                ..lens.clone()
            };
            barrier.wait().await;
            if create {
                repository.create(&candidate).await
            } else {
                repository.replace(&lens, &candidate).await
            }
        });
    }
    let mut results = Vec::new();
    while let Some(result) = tasks.join_next().await {
        results.push(result.unwrap());
    }
    results
}
