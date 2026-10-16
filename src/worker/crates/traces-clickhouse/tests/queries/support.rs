use std::collections::BTreeMap;

use litellm_traces::{DecodedSpan, decode_otlp};
use litellm_traces_clickhouse::{
    Connection, InsertTable, QueryReaders, ensure_schema, insert_rows,
};
use rstest::fixture;
use serde_json::{Value, json};

use crate::support::{ClickHouseDatabase, TestResult, database};

pub const DATABASE: &str = "trace_test";

pub struct SeededDatabase {
    pub database: ClickHouseDatabase,
    #[allow(dead_code)] // dead_code: also shared with reads.rs, which uses a direct storage reader
    pub readers: QueryReaders,
}

#[fixture]
pub async fn migrated_database(
    #[future(awt)] database: TestResult<ClickHouseDatabase>,
) -> TestResult<SeededDatabase> {
    let database = database?;
    let writer = Connection::writer(&database.url)?;
    ensure_schema(&database.client, &writer, DATABASE, 7).await?;
    for table in [
        "otel_traces",
        "agent_traces_by_key",
        "spend_logs",
        "lens_call_costs",
        "lens_trace_summaries",
    ] {
        database
            .client
            .post(writer.url().clone())
            .body(format!("ALTER TABLE {DATABASE}.{table} REMOVE TTL"))
            .send()
            .await?
            .error_for_status()?;
        database
            .client
            .post(writer.url().clone())
            .body(format!("SYSTEM STOP MERGES {DATABASE}.{table}"))
            .send()
            .await?
            .error_for_status()?;
    }
    let readers = QueryReaders::new(writer, DATABASE.to_owned());
    Ok(SeededDatabase { database, readers })
}

#[fixture]
pub async fn seeded_database(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
) -> TestResult<SeededDatabase> {
    let fixture = migrated_database?;
    let writer = Connection::writer(&fixture.database.url)?;
    for (contents, team, key) in [
        (
            include_bytes!("../../../traces/tests/fixtures/query_root.json").as_slice(),
            "team-a",
            "key-a",
        ),
        (
            include_bytes!("../../../traces/tests/fixtures/query_children.json").as_slice(),
            "team-a",
            "key-a",
        ),
        (
            include_bytes!("../../../traces/tests/fixtures/query_alternate.json").as_slice(),
            "team-a",
            "key-alt",
        ),
        (
            include_bytes!("../../../traces/tests/fixtures/query_other_team.json").as_slice(),
            "team-b",
            "key-b",
        ),
    ] {
        insert_export(&fixture, contents, team, key).await?;
    }
    let spend_rows = include_str!("../fixtures/spend_logs.jsonl")
        .lines()
        .map(serde_json::from_str::<BTreeMap<String, Value>>)
        .collect::<Result<Vec<_>, _>>()?;
    insert_rows(
        &fixture.database.client,
        &writer,
        DATABASE,
        InsertTable::SpendLogs,
        spend_rows,
    )
    .await?;
    Ok(fixture)
}

pub async fn insert_export(
    fixture: &SeededDatabase,
    contents: &[u8],
    team: &str,
    key: &str,
) -> TestResult<Vec<DecodedSpan>> {
    let spans = decode_otlp(contents, Some("application/json"))?;
    let writer = Connection::writer(&fixture.database.url)?;
    let rows = spans.iter().map(|span| span_row(span, team, key)).collect();
    insert_rows(
        &fixture.database.client,
        &writer,
        DATABASE,
        InsertTable::OtelTraces,
        rows,
    )
    .await?;
    Ok(spans)
}

#[allow(dead_code)]
pub async fn seed_interleaved_agent_runs(fixture: &SeededDatabase) -> TestResult<(i64, i64)> {
    let start_ms = time::OffsetDateTime::now_utc().unix_timestamp() * 1000 - 10_000;
    let rows = [
        ("a-old", "A", 1_000),
        ("b-old", "B", 2_000),
        ("a-middle", "A", 3_000),
        ("b-middle", "B", 4_000),
        ("a-new", "A", 5_000),
        ("b-new", "B", 6_000),
    ]
    .into_iter()
    .map(|(trace_id, agent, offset_ms)| {
        BTreeMap::from([
            (
                "Timestamp".into(),
                json!((start_ms + offset_ms) * 1_000_000),
            ),
            ("Duration".into(), json!(1_000_000)),
            ("TraceId".into(), json!(trace_id)),
            ("SpanId".into(), json!(format!("{trace_id}-span"))),
            ("ParentSpanId".into(), json!("")),
            ("SpanName".into(), json!(trace_id)),
            ("ObservationType".into(), json!("agent")),
            ("AgentName".into(), json!(agent)),
            ("TeamId".into(), json!("team-a")),
            ("ApiKeyHash".into(), json!("key-a")),
        ])
    })
    .collect();
    insert_rows(
        &fixture.database.client,
        &Connection::writer(&fixture.database.url)?,
        DATABASE,
        InsertTable::OtelTraces,
        rows,
    )
    .await?;
    Ok((start_ms, start_ms + 10_000))
}

fn span_row(span: &DecodedSpan, team: &str, key: &str) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("Timestamp".into(), json!(span.start_ns)),
        ("TraceId".into(), json!(span.trace_id)),
        ("SpanId".into(), json!(span.span_id)),
        ("ParentSpanId".into(), json!(span.parent_span_id)),
        ("TraceState".into(), json!(span.trace_state)),
        ("SpanName".into(), json!(span.name)),
        ("SpanKind".into(), json!(span.kind)),
        (
            "ServiceName".into(),
            json!(
                span.resource_attributes
                    .get("service.name")
                    .map(String::as_str)
                    .unwrap_or_default()
            ),
        ),
        ("ResourceAttributes".into(), json!(span.resource_attributes)),
        ("ScopeName".into(), json!(span.scope_name)),
        ("ScopeVersion".into(), json!(span.scope_version)),
        ("SpanAttributes".into(), json!(span.attributes)),
        ("Duration".into(), json!(span.end_ns - span.start_ns)),
        ("StatusCode".into(), json!(span.status_code)),
        ("StatusMessage".into(), json!(span.status_message)),
        ("TeamId".into(), json!(team)),
        ("ApiKeyHash".into(), json!(key)),
        (
            "ObservationType".into(),
            json!(span.normalized.observation_type),
        ),
        (
            "AgentName".into(),
            json!(span.normalized.agent_name.as_deref().unwrap_or_default()),
        ),
        (
            "Model".into(),
            json!(span.normalized.model.as_deref().unwrap_or_default()),
        ),
        (
            "LiteLLMRequestId".into(),
            json!(
                span.normalized
                    .calls
                    .key_set()
                    .into_iter()
                    .flatten()
                    .find_map(|key| match key {
                        litellm_traces::CallKey::LiteLlmRequest(id)
                        | litellm_traces::CallKey::ProviderResponse(id) => Some(id.as_str()),
                        litellm_traces::CallKey::ProviderRequest(_)
                        | litellm_traces::CallKey::Transport
                        | litellm_traces::CallKey::GatewayAttempt => None,
                    })
                    .unwrap_or_default()
            ),
        ),
        ("InputTokens".into(), json!(span.normalized.input_tokens)),
        ("OutputTokens".into(), json!(span.normalized.output_tokens)),
        ("Input".into(), json!(span.normalized.input)),
        ("Output".into(), json!(span.normalized.output)),
    ])
}
