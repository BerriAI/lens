use std::collections::BTreeMap;

use litellm_http::Client;
use litellm_storage_clickhouse::{ADHOC_QUERY_LIMITS, execute_read_with_limits};
use litellm_traces::{
    CodexContentState, CodexEventKind, CodexMessageRole, CodexProjection, DecodedLogRecord,
    LogListRequest, LogValue, QueryScope, Shared, Tenant,
};
use litellm_traces_clickhouse::{
    Connection, InsertTable, QueryReaders, ensure_schema, get_log, insert_shared_rows, list_logs,
    log_rows,
};
use rstest::rstest;
use serde_json::Value;
mod support;

use support::{ClickHouseDatabase, TestResult, database};

fn record(timestamp_ns: u64) -> DecodedLogRecord {
    DecodedLogRecord {
        timestamp_ns,
        effective_timestamp_ns: timestamp_ns,
        observed_timestamp_ns: timestamp_ns + 1,
        severity_number: 9,
        severity_text: "INFO".to_owned(),
        event_name: "source.rs:42".to_owned(),
        semantic_event_name: Some("codex.user_prompt".to_owned()),
        body: Some(LogValue::Bytes(vec![0xff])),
        attributes: BTreeMap::from([
            (
                "conversation.id".to_owned(),
                LogValue::String("conversation-one".to_owned()),
            ),
            ("attempt".to_owned(), LogValue::Integer(3)),
        ]),
        resource_attributes: Shared::new(BTreeMap::from([(
            "service.name".to_owned(),
            LogValue::String("codex_cli_rs".to_owned()),
        )])),
        scope_name: Shared::new("codex".to_owned()),
        scope_version: Shared::new("0.166.0".to_owned()),
        scope_attributes: Shared::new(BTreeMap::new()),
        resource_schema_url: String::new(),
        scope_schema_url: String::new(),
        trace_id: Vec::new(),
        span_id: Vec::new(),
        trace_flags: 0,
        dropped_attributes_count: 0,
        codex: Some(CodexProjection {
            event_kind: CodexEventKind::UserPrompt,
            message_role: Some(CodexMessageRole::User),
            content_state: Some(CodexContentState::Present),
            content: Some("prompt".to_owned()),
            conversation_id: Some("conversation-one".to_owned()),
            turn_id: None,
            item_id: None,
            call_id: None,
            tool_name: None,
            success: None,
            duration_ms: None,
            truncated: None,
        }),
    }
}

fn tenant() -> Tenant {
    Tenant {
        team_id: "team-1".to_owned(),
        api_key_hash: "key-1".to_owned(),
        org_id: "org-1".to_owned(),
        user_id: "user-1".to_owned(),
    }
}

async fn query(
    client: &Client,
    connection: &Connection,
    sql: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    let response = execute_read_with_limits(
        client,
        connection,
        sql,
        &BTreeMap::new(),
        ADHOC_QUERY_LIMITS,
    )
    .await?;
    Ok(serde_json::from_str(&response)?)
}

#[rstest]
#[tokio::test]
async fn generic_log_records_persist_typed_values_and_stable_identity(
    #[future(awt)] database: TestResult<ClickHouseDatabase>,
) -> TestResult {
    let database = database?;
    let writer = Connection::writer(&database.url)?;
    ensure_schema(&database.client, &writer, "trace_test", 7).await?;
    let timestamp = time::OffsetDateTime::now_utc().unix_timestamp_nanos() as u64;
    let records = log_rows(vec![record(timestamp)], &tenant())?
        .into_iter()
        .chain(log_rows(vec![record(timestamp + 2)], &tenant())?)
        .chain(log_rows(
            vec![record(timestamp + 1)],
            &Tenant {
                team_id: "team-2".to_owned(),
                api_key_hash: "key-2".to_owned(),
                org_id: "org-2".to_owned(),
                user_id: "user-2".to_owned(),
            },
        )?)
        .collect::<Vec<_>>();
    insert_shared_rows(
        &database.client,
        &writer,
        "trace_test",
        InsertTable::OtelLogs,
        records.clone(),
    )
    .await?;
    insert_shared_rows(
        &database.client,
        &writer,
        "trace_test",
        InsertTable::OtelLogs,
        records,
    )
    .await?;
    let rows = query(
        &database.client,
        &writer,
        "SELECT TeamId, RecordId, SemanticEventName, Framework, CodexProjection, ServiceName, ConversationId, TraceId, SpanId, Body, Attributes, ResourceAttributes \
         FROM trace_test.otel_logs FINAL WHERE SemanticEventName = 'codex.user_prompt' \
         AND ConversationId = 'conversation-one'",
    )
    .await?;
    assert_eq!(rows["data"].as_array().unwrap().len(), 3);
    let row = rows["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["TeamId"] == "team-1")
        .unwrap();
    assert_eq!(row["SemanticEventName"], "codex.user_prompt");
    assert_eq!(row["Framework"], "openai-codex");
    assert_eq!(
        serde_json::from_str::<Value>(row["CodexProjection"].as_str().unwrap()).unwrap()["event_kind"],
        "user_prompt"
    );
    assert_eq!(row["ServiceName"], "codex_cli_rs");
    assert_eq!(row["TraceId"], "");
    assert_eq!(
        serde_json::from_str::<Value>(row["Body"].as_str().unwrap()).unwrap(),
        serde_json::json!({"type": "bytes", "value": [255]})
    );
    assert_eq!(
        serde_json::from_str::<Value>(row["Attributes"].as_str().unwrap()).unwrap()["attempt"],
        serde_json::json!({"type": "integer", "value": 3})
    );
    let readers = QueryReaders::new(writer, "trace_test".to_owned());
    let reader = readers
        .connection(
            &database.client,
            &QueryScope::Owned {
                user_id: "user-1".to_owned(),
                team_ids: vec!["team-1".to_owned()],
            },
            "test-secret",
        )
        .await?;
    let scoped_rows = query(
        &database.client,
        &reader,
        "SELECT TeamId FROM trace_test.otel_logs FINAL ORDER BY TeamId",
    )
    .await?;
    assert_eq!(
        scoped_rows["data"],
        serde_json::json!([{"TeamId": "team-1"}, {"TeamId": "team-1"}])
    );
    let start_ms = (timestamp / 1_000_000) as i64 - 1;
    let end_ms = start_ms + 10;
    let first_page = list_logs(
        &database.client,
        &reader,
        LogListRequest {
            start_ms: Some(start_ms),
            end_ms: Some(end_ms),
            service: Some("codex_cli_rs".to_owned()),
            event: Some("codex.user_prompt".to_owned()),
            severity: None,
            trace_id: None,
            conversation_id: Some("conversation-one".to_owned()),
            cursor: None,
            page_size: Some(1),
        },
    )
    .await?;
    assert_eq!(first_page.data.len(), 1);
    assert!(first_page.next_cursor.is_some());
    let first_record = get_log(
        &database.client,
        &reader,
        first_page.data[0].record_id.clone(),
    )
    .await?
    .unwrap();
    assert_eq!(first_record.timestamp_ns, (timestamp + 2).to_string());
    assert_eq!(first_record.raw_timestamp_ns, (timestamp + 2).to_string());
    assert_eq!(
        first_record.observed_timestamp_ns,
        (timestamp + 3).to_string()
    );
    assert_eq!(first_record.attributes["attempt"], LogValue::Integer(3));
    let second_page = list_logs(
        &database.client,
        &reader,
        LogListRequest {
            start_ms: Some(start_ms),
            end_ms: Some(end_ms),
            service: Some("codex_cli_rs".to_owned()),
            event: Some("codex.user_prompt".to_owned()),
            severity: None,
            trace_id: None,
            conversation_id: Some("conversation-one".to_owned()),
            cursor: first_page.next_cursor,
            page_size: Some(1),
        },
    )
    .await?;
    assert_eq!(second_page.data.len(), 1);
    assert!(second_page.next_cursor.is_none());
    assert_ne!(first_page.data[0].record_id, second_page.data[0].record_id);
    Ok(())
}
