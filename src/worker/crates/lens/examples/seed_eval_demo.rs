//! Seeds the `moyai-regressions` dataset, its findings and source traces into a local Lens
//! ClickHouse so the eval API and Runs UI can be exercised end to end
//!
//! `CLICKHOUSE_URL=http://localhost:8123 CLICKHOUSE_DATABASE=lens LENS_DEMO_TEAM=moyai \
//!  cargo run -p litellm-lens --example seed_eval_demo`

use std::collections::BTreeMap;

use chrono::Utc;
use litellm_storage_clickhouse::{
    Connection,
    state::{Change, ClickHouseState, Snapshot},
};
use litellm_traces_clickhouse::{InsertTable, insert_rows};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Value, json};

const DATASET: &str = "moyai-regressions";
const REVISION: u64 = 7;
const CASES: [(&str, &str, &str, &str); 12] = [
    ("fix-flaky-retry-test", "Fix the flaky retry test in tests/test_router.py and open a PR", "high", "https://github.com/BerriAI/litellm"),
    ("bump-pydantic", "Bump pydantic to 2.11 and open a PR once the suite passes", "high", "https://github.com/BerriAI/litellm"),
    ("budget-off-by-one", "Budget alerts fire one request late. Fix it and open a PR", "high", "https://github.com/BerriAI/litellm"),
    ("rename-env-var", "Rename LENS_URL to LENS_BASE_URL everywhere and open a PR", "medium", "https://github.com/BerriAI/lens"),
    ("add-health-route", "Add a /health/live route to the worker and open a PR", "medium", "https://github.com/BerriAI/lens"),
    ("docs-typo", "Fix the typo in the deployment guide and open a PR", "low", "https://github.com/BerriAI/litellm-docs"),
    ("cache-ttl", "Make the cache TTL configurable and open a PR", "medium", "https://github.com/BerriAI/litellm"),
    ("null-model-crash", "The proxy crashes when model is null. Fix it and open a PR", "high", "https://github.com/BerriAI/litellm"),
    ("readme-badges", "Update the README badges and open a PR", "low", "https://github.com/BerriAI/lens"),
    ("timeout-default", "Change the default timeout to 600s and open a PR", "medium", "https://github.com/BerriAI/litellm"),
    ("lint-unused-imports", "Remove unused imports in litellm/proxy and open a PR", "low", "https://github.com/BerriAI/litellm"),
    ("retry-jitter", "Add jitter to router retries and open a PR", "medium", "https://github.com/BerriAI/litellm"),
];

fn key(namespace: &str, identity: Value) -> String {
    const CHARACTERS: &percent_encoding::AsciiSet = &NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'~');
    format!(
        "{namespace}/{}",
        utf8_percent_encode(&identity.to_string(), CHARACTERS)
    )
}

fn row(value: Value) -> BTreeMap<String, Value> {
    serde_json::from_value(value).expect("trace rows are objects")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("CLICKHOUSE_URL")?;
    let database = std::env::var("CLICKHOUSE_DATABASE").unwrap_or_else(|_| "lens".into());
    let team = std::env::var("LENS_DEMO_TEAM").unwrap_or_else(|_| "moyai".into());
    let http = litellm_lens::config::http_client()?;
    let writer = Connection::writer(&url)?;
    let state = ClickHouseState::new(http.clone(), Connection::reader(&url, &database)?);
    state.initialize(&format!("/lens/{database}")).await?;
    let now = Utc::now();
    let cases: Vec<Value> = CASES
        .iter()
        .enumerate()
        .map(|(index, (id, prompt, _, _))| {
            json!({
                "id": id,
                "messages": [{"role": "user", "content": prompt}],
                "reply": "Opened a PR with the change after the test suite passed",
                "expected": "Runs the tests before opening the PR",
                "source": {"trace_id": format!("prod-{id}"), "lens_id": "findings", "finding_id": (index + 1).to_string()},
                "agent_version": "4f1c2a9",
            })
        })
        .collect();
    let findings: Vec<Value> = CASES
        .iter()
        .enumerate()
        .map(|(index, (id, _, priority, _))| json!({"id": (index + 1).to_string(), "title": id, "priority": priority}))
        .collect();
    let summary = json!({
        "id": DATASET, "name": DATASET, "agent_name": "moyai", "revision": REVISION,
        "case_count": CASES.len(), "updated_at": now,
    });
    let changes = [
        (key("dataset-latest", json!([DATASET])), json!({"team_id": team, "summary": summary})),
        (
            key("dataset", json!([DATASET, REVISION])),
            json!({
                "id": DATASET, "name": DATASET, "agent_name": "moyai", "team_id": team,
                "created_at": now, "revision": REVISION, "created_by": "lens-demo", "cases": cases,
            }),
        ),
        (key("lens", json!(["findings"])), json!({"lens": {"scope": {"team_id": team}, "findings": findings}})),
    ];
    let mut commits = Vec::new();
    for (key, value) in changes {
        let previous = state
            .read_many(&[key.as_str()])
            .await?
            .pop()
            .unwrap_or_else(|| Snapshot::empty(key.clone()));
        commits.push(Change { previous, value });
    }
    state.commit(commits).await?;
    let timestamp = (now.timestamp_millis() - 86_400_000) * 1_000_000;
    let spans = CASES
        .iter()
        .map(|(id, prompt, _, repo)| {
            row(json!({
                "Timestamp": timestamp, "Duration": 1_000_000_000, "TraceId": format!("prod-{id}"),
                "SpanId": "root", "ParentSpanId": "", "SpanName": "moyai run", "StatusCode": "STATUS_CODE_OK",
                "TeamId": team, "ApiKeyHash": "moyai-prod",
                "ResourceAttributes": {"agent.name": "moyai", "agent.version": "4f1c2a9", "deployment.environment": "production"},
                "SpanAttributes": {"session.id": format!("prod-{id}"), "repo_url": repo},
                "Input": prompt, "Output": "Opened a PR",
            }))
        })
        .collect();
    insert_rows(&http, &writer, &database, InsertTable::OtelTraces, spans).await?;
    println!("seeded {DATASET}@{REVISION} with {} cases for team {team}", CASES.len());
    Ok(())
}
