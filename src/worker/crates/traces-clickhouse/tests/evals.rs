use std::collections::BTreeMap;

use lens_contract::eval::{TraceAttribute, TraceRef};
use litellm_traces_clickhouse::{
    Connection, InsertTable, ensure_schema, evals::EvalTraces, insert_rows,
};
use rstest::{fixture, rstest};
use serde_json::{Value, json};
use time::OffsetDateTime;

mod support;

use support::{ClickHouseDatabase, TestResult, database};

struct Fixture {
    database: ClickHouseDatabase,
    traces: EvalTraces,
    now_ms: i64,
}

#[fixture]
async fn populated(#[future] database: TestResult<ClickHouseDatabase>) -> TestResult<Fixture> {
    let database = database.await?;
    let writer = Connection::writer(&database.url)?;
    ensure_schema(&database.client, &writer, "eval_traces", 7).await?;
    let now = OffsetDateTime::now_utc();
    let timestamp = now.unix_timestamp_nanos() as i64;
    let now_ms = (now.unix_timestamp_nanos() / 1_000_000) as i64;
    let span = |team: &str, span: &str, parent: &str, input: &str, duration: u64| {
        serde_json::from_value::<BTreeMap<String, Value>>(json!({
            "Timestamp":timestamp,"TraceId":"trace-a","SpanId":span,"ParentSpanId":parent,
            "SpanName":span,"Duration":duration,"StatusCode":"STATUS_CODE_OK",
            "TeamId":team,"Input":input,"Output":"done","LiteLLMRequestId":"response-a",
            "ResourceAttributes":{"agent.name":"agent","agent.version":"sha","deployment.environment":"lens-eval"},
            "SpanAttributes":{"session.id":"session-a","gen_ai.tool.name":"run_tests","repo_url":input}
        }))
    };
    insert_rows(
        &database.client,
        &writer,
        "eval_traces",
        InsertTable::OtelTraces,
        vec![
            span("team-a", "root", "", "private-a", 0)?,
            span("team-a", "tool", "root", "tool-a", 1_000_000)?,
            span("team-b", "other-root", "", "private-b", 0)?,
        ],
    )
    .await?;
    let spend = |team: &str, id: &str, cost: f64| {
        serde_json::from_value::<BTreeMap<String, Value>>(json!({
            "request_id":id,"team_id":team,"trace_id":"trace-a","response_id":"response-a",
            "spend":cost,"start_time":now_ms,"end_time":now_ms
        }))
    };
    insert_rows(
        &database.client,
        &writer,
        "eval_traces",
        InsertTable::SpendLogs,
        vec![
            spend("team-a", "request-a", 0.25)?,
            spend("team-b", "request-b", 99.0)?,
        ],
    )
    .await?;
    let reader = Connection::configured(&database.url, "eval_traces", "default", "")?;
    Ok(Fixture {
        traces: EvalTraces::new(database.client.clone(), reader),
        database,
        now_ms,
    })
}

#[rstest]
#[case::same_team("team-a", Some("private-a"))]
#[case::other_team("team-b", Some("private-b"))]
#[case::unknown_team("team-c", None)]
#[tokio::test]
async fn source_root_attributes_are_team_scoped(
    #[future] populated: TestResult<Fixture>,
    #[case] team: &str,
    #[case] expected: Option<&str>,
) -> TestResult {
    let fixture = populated.await?;
    let attributes = fixture.traces.root_attributes(team, "trace-a").await?;
    assert_eq!(attributes.get("repo_url").map(String::as_str), expected);
    Ok(())
}

#[rstest]
#[case::session(TraceAttribute::SessionId, "session-a")]
#[case::trace(TraceAttribute::TraceId, "trace-a")]
#[tokio::test]
async fn eval_reads_keep_spans_and_spend_within_the_authenticated_team(
    #[future] populated: TestResult<Fixture>,
    #[case] attribute: TraceAttribute,
    #[case] value: &str,
) -> TestResult {
    let fixture = populated.await?;
    let trace = fixture
        .traces
        .read(
            "team-a",
            &TraceRef {
                attribute,
                value: value.into(),
            },
            fixture.now_ms + 60_000,
        )
        .await?
        .unwrap();
    assert_eq!(trace.spans.len(), 2);
    assert_eq!(trace.spans[0].input, "private-a");
    assert_eq!(trace.spans[1].attributes["gen_ai.tool.name"], "run_tests");
    assert_eq!(trace.gateway_cost_usd, 0.25);
    assert_eq!(trace.agent, "agent");
    assert_eq!(trace.version, "sha");
    assert_eq!(trace.environment, "lens-eval");
    assert!(trace.root_ended_at_ms.is_some());
    assert!(trace.last_received_at_ms >= fixture.now_ms);
    Ok(())
}

#[rstest]
#[case::unknown_team("team-c", "session-a", 60_000)]
#[case::unknown_session("team-a", "never-arrived", 60_000)]
#[case::before_receipt("team-a", "session-a", -60_000)]
#[tokio::test]
async fn missing_or_unreadable_traces_are_absent(
    #[future] populated: TestResult<Fixture>,
    #[case] team: &str,
    #[case] value: &str,
    #[case] offset_ms: i64,
) -> TestResult {
    let fixture = populated.await?;
    assert!(
        fixture
            .traces
            .read(
                team,
                &TraceRef {
                    attribute: TraceAttribute::SessionId,
                    value: value.into()
                },
                fixture.now_ms + offset_ms
            )
            .await?
            .is_none()
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn session_reference_includes_every_trace_in_the_session(
    #[future] populated: TestResult<Fixture>,
) -> TestResult {
    let fixture = populated.await?;
    let writer = Connection::writer(&fixture.database.url)?;
    insert_rows(&fixture.database.client, &writer, "eval_traces", InsertTable::OtelTraces,
        vec![serde_json::from_value(json!({"Timestamp":OffsetDateTime::now_utc().unix_timestamp_nanos() as i64,
            "TraceId":"different-trace","SpanId":"root-two","TeamId":"team-a",
            "ResourceAttributes":{"agent.name":"agent","agent.version":"sha","deployment.environment":"lens-eval"},
            "SpanAttributes":{"session.id":"session-a"}}))?]).await?;
    let trace = fixture
        .traces
        .read(
            "team-a",
            &TraceRef {
                attribute: TraceAttribute::SessionId,
                value: "session-a".into(),
            },
            fixture.now_ms + 60_000,
        )
        .await?
        .unwrap();
    assert_eq!(trace.spans.len(), 3);
    assert!(trace.spans.iter().any(|span| span.span_id == "root-two"));
    assert!(trace.root_ended_at_ms.is_some());
    assert_eq!(trace.agent, "agent");
    assert_eq!(trace.gateway_cost_usd, 0.25);
    Ok(())
}
