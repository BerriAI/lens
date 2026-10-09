#[path = "support/database.rs"]
mod storage_support;
mod support;

use std::collections::BTreeMap;

use lens_auth::ingestion::IngestionRepository;
use lens_contract::{feedback::TraceIdentity, investigations::Scope, worker::Job};
use lens_datasets::DatasetRepository;
use lens_investigations::{LensRepository, WorkerRepository};
use lens_migrate::{Error, import, read_source};
use lens_signals::SignalRepository;
use litellm_http::Client;
use litellm_storage_clickhouse::{
    Parameter, datasets::Datasets, execute_read, ingestion::IngestionKeys,
    investigations::Investigations, signals::Signals,
};
use rstest::rstest;
use serde_json::{Value, json};
use sqlx::{Connection, PgConnection};
use storage_support::{Database, database};
use support::{plan, source};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{ImageExt, runners::AsyncRunner},
};

async fn seed(connection: &mut PgConnection, source: &Value) {
    sqlx::raw_sql(include_str!("support/postgres.sql"))
        .execute(&mut *connection)
        .await
        .unwrap();
    let tables = [
        (
            "lenses",
            "INSERT INTO \"LiteLLM_Lens\" SELECT * FROM jsonb_populate_recordset(NULL::\"LiteLLM_Lens\", $1)",
        ),
        (
            "runs",
            "INSERT INTO \"LiteLLM_LensRun\" SELECT * FROM jsonb_populate_recordset(NULL::\"LiteLLM_LensRun\", $1)",
        ),
        (
            "reviews",
            "INSERT INTO \"LiteLLM_LensReview\" SELECT * FROM jsonb_populate_recordset(NULL::\"LiteLLM_LensReview\", $1)",
        ),
        (
            "workers",
            "INSERT INTO \"LiteLLM_LensWorker\" SELECT * FROM jsonb_populate_recordset(NULL::\"LiteLLM_LensWorker\", $1)",
        ),
        (
            "ingestion_keys",
            "INSERT INTO \"LiteLLM_LensIngestionKey\" SELECT * FROM jsonb_populate_recordset(NULL::\"LiteLLM_LensIngestionKey\", $1)",
        ),
        (
            "datasets",
            "INSERT INTO \"LiteLLM_LensDataset\" SELECT * FROM jsonb_populate_recordset(NULL::\"LiteLLM_LensDataset\", $1)",
        ),
        (
            "signal_configs",
            "INSERT INTO \"LiteLLM_LensSignalConfig\" SELECT * FROM jsonb_populate_recordset(NULL::\"LiteLLM_LensSignalConfig\", $1)",
        ),
        (
            "trace_signals",
            "INSERT INTO \"LiteLLM_LensTraceSignal\" SELECT * FROM jsonb_populate_recordset(NULL::\"LiteLLM_LensTraceSignal\", $1)",
        ),
    ];
    for (table, query) in tables {
        sqlx::query(query)
            .bind(&source[table])
            .execute(&mut *connection)
            .await
            .unwrap();
    }
}

#[rstest]
#[tokio::test]
async fn read_only_postgres_snapshot_becomes_usable_by_rust_lens(
    #[future(awt)] database: Database,
    mut source: Value,
) {
    source["lenses"][0]["due_at"] = json!("2026-01-15T01:02:03.123");
    let postgres = Postgres::default()
        .with_tag(
            "16-alpine@sha256:721873c34ceb9f8d8fc265984940dc982404c105f19ad51be9fdc5970a6080ea",
        )
        .start()
        .await
        .unwrap();
    let host = postgres.get_host().await.unwrap();
    let port = postgres.get_host_port_ipv4(5432).await.unwrap();
    let mut connection = PgConnection::connect(&format!(
        "postgres://postgres:postgres@{host}:{port}/postgres"
    ))
    .await
    .unwrap();
    seed(&mut connection, &source).await;
    let url = format!("postgres://migration_reader:fixture-reader@{host}:{port}/postgres");
    let expected = plan(source.clone()).unwrap();
    let imported = read_source(&url).await.unwrap().plan().unwrap();
    assert_eq!(imported.records(), expected.records());
    assert_eq!(imported.report(), expected.report());
    let planned = std::process::Command::new(env!("CARGO_BIN_EXE_lens-migrate"))
        .env_clear()
        .env("LENS_MIGRATION_POSTGRES_URL", &url)
        .output()
        .unwrap();
    assert!(
        planned.status.success(),
        "{}",
        String::from_utf8_lossy(&planned.stderr)
    );
    let planned: Value = serde_json::from_slice(&planned.stdout).unwrap();
    assert_eq!(planned["applied"], false);
    assert_eq!(planned["verified"], false);
    assert_eq!(
        planned["plan"],
        serde_json::to_value(expected.report()).unwrap()
    );
    import(&database.state, &imported, &database.keeper)
        .await
        .unwrap();
    let applied = std::process::Command::new(env!("CARGO_BIN_EXE_lens-migrate"))
        .args(["--apply", "--source-stopped"])
        .env_clear()
        .env("LENS_MIGRATION_POSTGRES_URL", &url)
        .env("CLICKHOUSE_URL", database.writer.url().as_str())
        .env("CLICKHOUSE_DATABASE", &database.name)
        .output()
        .unwrap();
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    let applied: Value = serde_json::from_slice(&applied.stdout).unwrap();
    assert_eq!(applied["applied"], true);
    assert_eq!(applied["verified"], true);
    assert_eq!(applied["plan"], planned["plan"]);
    assert_eq!(
        database.values(expected.records().keys()).await,
        *expected.records()
    );
    let repository = Investigations(database.state.clone());
    let lens = repository.get("lens").await.unwrap().unwrap();
    assert_eq!(
        lens.version,
        source["lenses"][0]["data"]["version"].as_i64().unwrap()
    );
    assert_eq!(lens.findings.len(), 1);
    assert_eq!(
        repository
            .lenses(&Scope {
                team_id: "other".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .len(),
        0
    );
    assert_eq!(repository.lenses(&lens.scope).await.unwrap().len(), 1);
    let archived: Job = serde_json::from_value(source["runs"][0]["data"].clone()).unwrap();
    assert_eq!(
        repository.job("lens", "archive").await.unwrap().unwrap().id,
        archived.id
    );
    assert_eq!(repository.jobs("lens", 0).await.unwrap().len(), 2);
    assert_eq!(
        repository.reviews("lens", &archived).await.unwrap().len(),
        1
    );
    assert_eq!(
        repository
            .worker("keep-hash-byte-exact")
            .await
            .unwrap()
            .unwrap()
            .id,
        source["workers"][0]["id"]
    );
    assert_eq!(
        IngestionKeys(database.state.clone()).list().await.unwrap()[0]
            .tenant
            .api_key_hash,
        source["ingestion_keys"][0]["data"]["tenant"]["api_key_hash"]
            .as_str()
            .unwrap()
    );
    assert_eq!(
        Datasets(database.state.clone())
            .get("dataset", None)
            .await
            .unwrap()
            .unwrap()
            .revision,
        2
    );
    assert_eq!(
        Datasets(database.state.clone())
            .get("dataset", Some(1))
            .await
            .unwrap()
            .unwrap()
            .revision,
        1
    );
    assert_eq!(
        Signals(database.state.clone())
            .traces(&[TraceIdentity {
                trace_id: "trace".into(),
                trace_ref: "backend-key".into()
            }])
            .await
            .unwrap()[0]
            .config_key,
        "keep-config-key"
    );
    assert_eq!(
        read_source(&url).await.unwrap().plan().unwrap().report(),
        expected.report()
    );
}

#[rstest]
#[case::malformed_hash(false)]
#[case::duplicate_hash(true)]
#[tokio::test]
async fn invalid_ingestion_credentials_refuse_apply_before_target_writes(
    #[future(awt)] database: Database,
    mut source: Value,
    #[case] duplicate: bool,
) {
    let expected = if duplicate {
        let mut second = source["ingestion_keys"][0].clone();
        second["id"] = json!("second-key");
        second["data"]["id"] = json!("second-key");
        second["data"]["tenant"]["team_id"] = json!("another-team");
        source["ingestion_keys"]
            .as_array_mut()
            .unwrap()
            .push(second);
        Error::Duplicate
    } else {
        source["ingestion_keys"][0]["data"]["tenant"]["api_key_hash"] = json!("invalid-token-hash");
        Error::InvalidRecord
    };
    let postgres = Postgres::default()
        .with_tag(
            "16-alpine@sha256:721873c34ceb9f8d8fc265984940dc982404c105f19ad51be9fdc5970a6080ea",
        )
        .start()
        .await
        .unwrap();
    let host = postgres.get_host().await.unwrap();
    let port = postgres.get_host_port_ipv4(5432).await.unwrap();
    let mut connection = PgConnection::connect(&format!(
        "postgres://postgres:postgres@{host}:{port}/postgres"
    ))
    .await
    .unwrap();
    seed(&mut connection, &source).await;
    let url = format!("postgres://migration_reader:fixture-reader@{host}:{port}/postgres");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_lens-migrate"))
        .args(["--apply", "--source-stopped"])
        .env_clear()
        .env("LENS_MIGRATION_POSTGRES_URL", &url)
        .env("CLICKHOUSE_URL", database.writer.url().as_str())
        .env("CLICKHOUSE_DATABASE", &database.name)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap().trim(),
        expected.to_string()
    );
    let tables: Value = serde_json::from_str(
        &execute_read(
            &Client::no_redirect_for_test(),
            &database.writer,
            "SELECT name FROM system.tables WHERE database = {database:String}",
            &BTreeMap::from([("database".into(), Parameter::Text(database.name.clone()))]),
        )
        .await
        .unwrap(),
    )
    .unwrap();
    assert!(tables["data"].as_array().unwrap().is_empty());
}
