use std::{
    future::Future,
    time::{Duration, Instant},
};

use litellm_http::Client;
use litellm_storage_clickhouse::{
    Connection,
    state::{Change, ClickHouseState},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const KEYS: usize = 1000;
const REVISIONS: usize = 20;
const BATCH: usize = 100;

fn value(key: &str, revision: usize) -> Value {
    let payload: String = (0..256)
        .map(|block| format!("{:x}", Sha256::digest(format!("{key}/{revision}/{block}"))))
        .collect();
    json!({"revision": revision, "payload": payload})
}

async fn measured<T>(
    operation: &'static str,
    samples: &mut Vec<Value>,
    future: impl Future<Output = T>,
) -> T {
    let start = Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(30), future)
        .await
        .expect("public storage operation exceeds the predeclared 30-second limit");
    samples.push(
        json!({"operation": operation, "elapsed_ms": start.elapsed().as_secs_f64() * 1000.0}),
    );
    result
}

async fn sql(client: &Client, connection: &Connection, statement: &str) -> String {
    let response = client
        .post(connection.url().clone())
        .body(statement.to_owned())
        .send()
        .await
        .unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert!(status.is_success(), "{text}");
    text
}

async fn counts(client: &Client, connection: &Connection) -> Value {
    let result = sql(
        client,
        connection,
        "SELECT (SELECT count() FROM lens_state_heads) AS heads, \
         (SELECT count() FROM lens_state_blobs FINAL) AS blobs, \
         (SELECT sum(length(data)) FROM lens_state_blobs FINAL) AS logical_bytes, \
         sumIf(bytes_on_disk, active) AS active_bytes, \
         sumIf(bytes_on_disk, NOT active) AS inactive_bytes \
         FROM system.parts WHERE database=currentDatabase() AND table='lens_state_blobs' \
         FORMAT JSONEachRow SETTINGS output_format_json_quote_64bit_integers=0",
    )
    .await;
    serde_json::from_str(&result).unwrap()
}

#[tokio::main]
async fn main() {
    let url = std::env::var("CLICKHOUSE_STATE_TEST_URL").unwrap();
    let output = std::env::var("LENS_STATE_CAPACITY_OUTPUT").unwrap();
    let client = Client::no_redirect_for_test();
    let database = format!("lens_capacity_{}", uuid::Uuid::new_v4().simple());
    let admin = Connection::writer(&url).unwrap();
    sql(&client, &admin, &format!("CREATE DATABASE `{database}`")).await;
    let connection = Connection::reader(&url, &database).unwrap();
    let store = ClickHouseState::new(client.clone(), connection.clone());
    store
        .initialize(&format!("/state-capacity/{database}"))
        .await
        .unwrap();
    let keys: Vec<_> = (0..KEYS).map(|key| format!("capacity/{key:04}")).collect();
    let mut samples = Vec::new();
    let mut passes = Vec::new();
    let start = Instant::now();
    for revision in 1..=REVISIONS {
        for batch in keys.chunks(BATCH) {
            let references: Vec<_> = batch.iter().map(String::as_str).collect();
            let previous = measured("read", &mut samples, store.read_many(&references))
                .await
                .unwrap();
            let changes = previous
                .into_iter()
                .map(|previous| Change {
                    value: value(&previous.head.key, revision),
                    previous,
                })
                .collect();
            measured("commit", &mut samples, store.commit(changes))
                .await
                .unwrap();
        }
        eprintln!("revision {revision}/{REVISIONS} committed");
        if revision % 5 != 0 {
            continue;
        }
        let before = counts(&client, &connection).await;
        for pass in 1..=2 {
            for batch in keys.chunks(BATCH) {
                let references: Vec<_> = batch.iter().map(String::as_str).collect();
                measured("compact", &mut samples, store.compact(&references))
                    .await
                    .unwrap();
                let current = measured("read", &mut samples, store.read_many(&references))
                    .await
                    .unwrap();
                assert_eq!(current.len(), BATCH);
                for (key, snapshot) in batch.iter().zip(current) {
                    assert_eq!(&snapshot.head.key, key);
                    assert_eq!(snapshot.value, value(key, revision));
                }
            }
            let after = counts(&client, &connection).await;
            assert_eq!(after["heads"], KEYS);
            assert_eq!(after["blobs"], KEYS);
            assert!(after["active_bytes"].as_u64().unwrap() <= 98_304_000);
            passes.push(
                json!({"revision": revision, "pass": pass, "before": before, "after": after}),
            );
            eprintln!("revision {revision}, compaction pass {pass}: {after}");
        }
    }
    let distributions: Vec<_> = ["read", "commit", "compact"].into_iter().map(|operation| {
        let mut values: Vec<_> = samples.iter().filter(|sample| sample["operation"] == operation)
            .map(|sample| sample["elapsed_ms"].as_f64().unwrap()).collect();
        values.sort_by(f64::total_cmp);
        json!({"operation": operation, "count": values.len(), "p50_ms": values[values.len() / 2],
            "p95_ms": values[(values.len() * 95).div_ceil(100) - 1],
            "p99_ms": values[(values.len() * 99).div_ceil(100) - 1], "max_ms": values.last()})
    }).collect();
    let report = json!({"status": "passed", "database": database, "keys": KEYS,
        "revisions": REVISIONS, "successful_record_writes": KEYS * REVISIONS,
        "elapsed_seconds": start.elapsed().as_secs_f64(), "passes": passes,
        "latency": distributions, "samples": samples});
    std::fs::write(output, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    sql(&client, &admin, &format!("DROP DATABASE `{database}` SYNC")).await;
}
