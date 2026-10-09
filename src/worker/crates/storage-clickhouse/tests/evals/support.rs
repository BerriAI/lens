#[path = "../persistence/mod.rs"]
mod persistence;

pub use persistence::{Database, database, isolated_database, now};

use std::collections::BTreeMap;

use chrono::Duration;
use lens_contract::eval::{CaseResult, CreateEvalRun, EvalRun, RunStatus};
use lens_evals::{Evaluation, RunRepository, StoredRun};
use litellm_storage_clickhouse::evals::Evals;
use rstest::fixture;
use serde_json::json;

impl Database {
    pub fn repository(&self) -> Evals {
        Evals(self.independent())
    }
}

#[fixture]
pub fn run() -> StoredRun {
    let spec: CreateEvalRun = serde_json::from_value(json!({
        "eval":"support", "agent":"support-agent", "dataset_id":"dataset", "revision":1,
        "version":"sha-1", "branch":"main", "trials":2, "scorers":[{"kind":"task_completed"}]
    }))
    .unwrap();
    StoredRun {
        run: EvalRun {
            id: "run 雪/'\"".into(),
            status: RunStatus::Running,
            eval: spec.eval.clone(),
            agent: spec.agent.clone(),
            version: spec.version.clone(),
            branch: spec.branch.clone(),
            pr: None,
            url: "http://lens.local/run".into(),
            expected_trials: 4,
            received_trials: 0,
            summary: None,
            failure: String::new(),
        },
        spec,
        cases: serde_json::from_value(json!([
            {"id":"case-a","messages":[{"role":"user","content":"First"}],"source":{}},
            {"id":"case-b","messages":[{"role":"user","content":"Second"}],"source":{}}
        ]))
        .unwrap(),
        team_id: "team".into(),
        version: 0,
        created_at: now(),
        completed_at: None,
        submissions: vec![],
        verdicts: BTreeMap::new(),
        resolved_traces: BTreeMap::new(),
        lease: None,
    }
}

#[fixture]
pub fn result() -> CaseResult {
    serde_json::from_value(json!({"trace":{"value":"session"},"cost_usd":0.25,"duration_ms":100}))
        .unwrap()
}

#[fixture]
pub fn evaluation() -> Evaluation {
    Evaluation {
        summary:serde_json::from_value(json!({
            "passed":2,"total":2,"pass_rate":1.0,"cost_per_case":0.5,"scores":{"task_completed":1.0},
            "errors":0,"baseline_run_id":null,"baseline_version":null,"regressions":[],"fixed":[],
            "gate":{"passed":true,"reasons":[]}
        })).unwrap(),
        verdicts:BTreeMap::from([("case-a".into(),true),("case-b".into(),true)]),
        resolved_traces: BTreeMap::from([("case-a".into(), vec![lens_contract::feedback::TraceIdentity {
            trace_id: "resolved-trace".into(), trace_ref: "trace-reference".into()
        }])]),
    }
}

pub fn record_key(namespace: &str, parts: &[&str]) -> String {
    const ENCODING: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'.')
        .remove(b'_')
        .remove(b'~');
    format!(
        "{namespace}/{}",
        percent_encoding::utf8_percent_encode(&serde_json::to_string(parts).unwrap(), ENCODING)
    )
}

pub async fn scoring(database: &Database, run: &StoredRun) -> StoredRun {
    let repository = database.repository();
    repository.create("key", run).await.unwrap();
    repository.finish(&run.run.id).await.unwrap();
    repository
        .claim(&run.run.id, "worker", now(), now() + Duration::minutes(5))
        .await
        .unwrap()
        .unwrap()
}

pub async fn create_concurrently(database: &Database, run: &StoredRun) -> Vec<StoredRun> {
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(16));
    let mut tasks = tokio::task::JoinSet::new();
    for index in 0..16 {
        let repository = database.repository();
        let barrier = barrier.clone();
        let candidate = StoredRun {
            run: EvalRun {
                id: format!("run-{index}"),
                ..run.run.clone()
            },
            ..run.clone()
        };
        tasks.spawn(async move {
            barrier.wait().await;
            repository.create("key", &candidate).await.unwrap()
        });
    }
    tasks.join_all().await
}

pub async fn submit_concurrently(database: &Database, run: &StoredRun) {
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(4));
    let mut tasks = tokio::task::JoinSet::new();
    for index in 0..4 {
        let repository = database.repository();
        let barrier = barrier.clone();
        let id = run.run.id.clone();
        tasks.spawn(async move {
            barrier.wait().await;
            repository
                .submit(
                    &id,
                    if index < 2 { "case-a" } else { "case-b" },
                    index % 2,
                    &result(),
                    now(),
                )
                .await
                .unwrap();
        });
    }
    tasks.join_all().await;
}

pub async fn claim_concurrently(database: &Database, id: &str) -> Vec<Option<StoredRun>> {
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(16));
    let mut tasks = tokio::task::JoinSet::new();
    for index in 0..16 {
        let repository = database.repository();
        let barrier = barrier.clone();
        let id = id.to_owned();
        tasks.spawn(async move {
            barrier.wait().await;
            repository
                .claim(
                    &id,
                    &format!("worker-{index}"),
                    now(),
                    now() + Duration::minutes(5),
                )
                .await
                .unwrap()
        });
    }
    tasks.join_all().await
}

pub async fn seed_runs(database: &Database, run: &StoredRun) -> Vec<StoredRun> {
    use litellm_storage_clickhouse::state::{Change, Snapshot};
    let mut runs: Vec<_> = (0..130)
        .map(|index| StoredRun {
            run: EvalRun {
                id: format!("run-{index:03}"),
                ..run.run.clone()
            },
            ..run.clone()
        })
        .chain(["a", "z", "λ"].map(|id| StoredRun {
            run: EvalRun {
                id: id.into(),
                ..run.run.clone()
            },
            ..run.clone()
        }))
        .collect();
    database
        .store
        .commit(
            runs.iter()
                .map(|run| Change {
                    previous: Snapshot::empty(record_key("eval-run", &[&run.run.id])),
                    value: serde_json::to_value(run).unwrap(),
                })
                .collect(),
        )
        .await
        .unwrap();
    runs.sort_by(|left, right| left.run.id.cmp(&right.run.id));
    runs
}
