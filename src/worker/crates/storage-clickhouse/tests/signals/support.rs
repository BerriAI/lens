#[path = "../persistence/mod.rs"]
mod persistence;

pub use persistence::{Database, database, isolated_database, now};

use lens_contract::{feedback::TraceIdentity, signals::SignalConfig, worker::Execution};
use lens_signals::SignalRepository;
use litellm_storage_clickhouse::signals::Signals;
use rstest::fixture;
use serde_json::{Value, json};

pub const CONFIG: &str = "signals/config";

impl Database {
    pub fn repository(&self) -> Signals {
        Signals(self.independent())
    }
}

#[fixture]
pub fn config() -> SignalConfig {
    serde_json::from_value(json!({"model": "decision", "threshold": 0.7})).unwrap()
}

#[fixture]
pub fn execution() -> Execution {
    let recorded: Value = serde_json::from_str(include_str!(
        "../../../contract/tests/fixtures/investigations_public.json"
    ))
    .unwrap();
    Execution {
        trace_id: "trace 雪/'\"".into(),
        trace_ref: "ref 雪/'\"".into(),
        span_count: 2,
        ..serde_json::from_value(recorded["sample"]["executions"][0].clone()).unwrap()
    }
}

pub fn identity(execution: &Execution) -> TraceIdentity {
    TraceIdentity {
        trace_id: execution.trace_id.clone(),
        trace_ref: execution.trace_ref.clone(),
    }
}

pub fn key(execution: &Execution) -> String {
    const ENCODING: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'.')
        .remove(b'_')
        .remove(b'~');
    format!(
        "trace-signal/{}",
        percent_encoding::utf8_percent_encode(
            &serde_json::to_string(&[&execution.trace_id, &execution.trace_ref]).unwrap(),
            ENCODING
        )
    )
}

pub fn value(value: impl serde::Serialize) -> Value {
    serde_json::to_value(value).unwrap()
}

pub async fn concurrent_claims(database: &Database) -> Vec<bool> {
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(16));
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..16 {
        let repository = database.repository();
        let barrier = barrier.clone();
        tasks.spawn(async move {
            barrier.wait().await;
            repository
                .claim(
                    &execution(),
                    &config(),
                    now() + chrono::Duration::minutes(5),
                    now(),
                )
                .await
                .unwrap()
        });
    }
    let mut results = Vec::new();
    while let Some(result) = tasks.join_next().await {
        results.push(result.unwrap());
    }
    results
}
