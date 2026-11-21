use litellm_storage_clickhouse::fetch;
use litellm_traces_clickhouse::{
    Connection, InsertTable, insert_rows,
    query::alert_rules::{AlertRuleCandidates, AlertRuleParams},
};
use rstest::rstest;
use serde_json::json;
use std::collections::BTreeMap;

#[path = "queries/support.rs"]
mod fixtures;
mod support;
use fixtures::{SeededDatabase, migrated_database};
use support::TestResult;

#[rstest]
#[tokio::test]
async fn candidates_use_elapsed_time_and_model_error_status_with_scoped_identity(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
) -> TestResult {
    let fixture = migrated_database?;
    let now = time::OffsetDateTime::now_utc().unix_timestamp() * 1000;
    let start = now - 600_000;
    let rows = [
        ("long", "a", "root", "", "agent", 0, 301_000, "STATUS_CODE_OK"),
        ("boundary", "a", "root", "", "agent", 0, 300_000, "STATUS_CODE_OK"),
        ("shared-id", "a", "failed-model", "root", "llm", 0, 100, "STATUS_CODE_ERROR"),
        ("shared-id", "b", "ok-model", "root", "llm", 0, 100, "STATUS_CODE_OK"),
        ("tool-error", "a", "failed-tool", "root", "tool", 0, 100, "STATUS_CODE_ERROR"),
        ("growing", "a", "early", "root", "llm", 0, 100, "STATUS_CODE_OK"),
        ("growing", "a", "late", "root", "llm", 301_000, 100, "STATUS_CODE_OK"),
        ("old", "a", "root", "", "agent", -600_000, 900_000, "STATUS_CODE_OK"),
    ].into_iter().map(|(trace, team, span, parent, kind, offset, duration, status)| {
        serde_json::from_value::<BTreeMap<String, serde_json::Value>>(json!({
            "TraceId":trace,"TeamId":team,"ApiKeyHash":"example-key-hash","SpanId":span,"ParentSpanId":parent,
            "SpanName":span,"ObservationType":kind,"Timestamp":(start + offset) * 1_000_000,
            "Duration":duration * 1_000_000_i64,"StatusCode":status,
        })).unwrap()
    }).collect();
    insert_rows(
        &fixture.database.client,
        &Connection::writer(&fixture.database.url)?,
        fixtures::DATABASE,
        InsertTable::OtelTraces,
        rows,
    )
    .await?;
    let connection =
        Connection::configured(&fixture.database.url, fixtures::DATABASE, "default", "")?;
    let parameters = AlertRuleParams {
        since_ms: start,
        now_ms: now,
        duration_ms: 300_000,
        model_errors: true,
        after: String::new(),
        limit: 100,
    };
    let rows =
        fetch::<AlertRuleCandidates>(&fixture.database.client, &connection, &parameters).await?;
    let found: BTreeMap<_, _> = rows
        .iter()
        .map(|row| {
            (
                row.trace_id.as_str(),
                (row.duration_ms, row.error_span_id.as_str()),
            )
        })
        .collect();
    assert_eq!(
        found,
        BTreeMap::from([
            ("long", (301_000, "")),
            ("growing", (301_100, "")),
            ("shared-id", (100, "failed-model")),
            ("tool-error", (100, "")),
        ])
    );
    let growing = rows.iter().find(|row| row.trace_id == "growing").unwrap();
    assert_eq!(growing.duration_span_id, "late");
    let last = rows.last().unwrap();
    let page = fetch::<AlertRuleCandidates>(
        &fixture.database.client,
        &connection,
        &AlertRuleParams {
            after: last.trace_ref.clone(),
            ..parameters
        },
    )
    .await?;
    assert!(page.is_empty());
    Ok(())
}
