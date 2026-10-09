use std::time::Duration;

use chrono::{DateTime, Utc};
use lens_contract::datasets::Dataset;
use lens_datasets::{DatasetRepository, StoreError, StoredSummary};
use litellm_http::Client;
use litellm_storage_clickhouse::{
    Connection,
    datasets::Datasets,
    execute_statement,
    state::{Change, ClickHouseState, Snapshot},
};
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
                    let result = execute_statement(
                        &Client::no_redirect_for_test(),
                        &connection,
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
    pub fn repository(&self) -> Datasets {
        Datasets(ClickHouseState::new(
            Client::no_redirect_for_test(),
            self.connection.clone(),
        ))
    }

    pub async fn seed(&self, key: &str, value: Value) {
        self.store.commit(vec![initial(key, value)]).await.unwrap();
    }

    pub async fn restart(&self) -> Datasets {
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
                if store.heads(&["dataset"]).await.is_ok() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
        Datasets(store)
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
    let name = format!("lens_rust_datasets_{}", uuid::Uuid::new_v4().simple());
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
        .initialize(&format!("/datasets-tests/{name}"))
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
pub fn saved_at() -> DateTime<Utc> {
    "2026-03-01T12:00:00.123Z".parse().unwrap()
}

pub fn dataset(id: &str, revision: i64, case_count: usize, team_id: &str) -> Dataset {
    serde_json::from_value(json!({
        "id": id,
        "name": format!("Dataset r{revision}"),
        "agent_name": "support-agent",
        "team_id": team_id,
        "created_at": saved_at(),
        "revision": revision,
        "created_by": "user-1",
        "cases": (0..case_count).map(|index| json!({
            "id": format!("case-{revision}-{index}"),
            "messages": [{"role": "user", "content": format!("question {index}")}],
            "reply": format!("answer {index}"),
            "source": {"trace_id": format!("trace-{index}")},
        })).collect::<Vec<_>>()
    }))
    .unwrap()
}

pub fn initial(key: &str, value: Value) -> Change {
    Change {
        previous: Snapshot::empty(key),
        value,
    }
}

#[fixture]
pub fn summaries(saved_at: DateTime<Utc>) -> Vec<StoredSummary> {
    (0..130)
        .map(|index| {
            serde_json::from_value(json!({
                "team_id": "team-a",
                "summary": {
                    "id": format!("dataset-{index:03}"),
                    "name": "Saved",
                    "agent_name": "agent",
                    "revision": 1,
                    "case_count": index,
                    "updated_at": saved_at + chrono::Duration::seconds(index)
                }
            }))
            .unwrap()
        })
        .collect()
}

#[fixture]
pub fn same_revision() -> Vec<Dataset> {
    (0..16)
        .map(|count| dataset("concurrent", 1, count, "team-a"))
        .collect()
}

#[fixture]
pub fn distinct_revisions() -> Vec<Dataset> {
    (1..=16)
        .map(|revision| dataset("concurrent", revision, 1, "team-a"))
        .collect()
}

pub async fn seed_legacy_records(database: &Database) {
    let records: std::collections::BTreeMap<String, Value> =
        serde_json::from_str(include_str!("fixtures/legacy_records.json")).unwrap();
    database
        .store
        .commit(
            records
                .into_iter()
                .map(|(key, value)| initial(&key, value))
                .collect(),
        )
        .await
        .unwrap();
}

pub async fn insert_concurrently(
    database: &Database,
    datasets: Vec<Dataset>,
    saved_at: DateTime<Utc>,
) -> Vec<(Dataset, Result<bool, StoreError>)> {
    let mut tasks = tokio::task::JoinSet::new();
    for dataset in datasets {
        let repository = database.repository();
        tasks.spawn(async move {
            let result = repository.insert(&dataset, saved_at).await;
            (dataset, result)
        });
    }
    tasks.join_all().await
}

pub async fn revisions(repository: &Datasets, id: &str, count: i64) -> Vec<Option<Dataset>> {
    let mut records = Vec::new();
    for revision in 1..=count {
        records.push(repository.get(id, Some(revision)).await.unwrap());
    }
    records
}
