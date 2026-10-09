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
            "TeamId":team,"ApiKeyHash":"key-a","UserId":"owner-a","Input":input,"Output":"done","LiteLLMRequestId":"response-a",
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
            span("", "root", "", "private-default", 0)?,
            span("", "tool", "root", "tool-default", 1_000_000)?,
            span("team-a", "root", "", "private-a", 0)?,
            span("team-a", "tool", "root", "tool-a", 1_000_000)?,
            span("team-b", "other-root", "", "private-b", 0)?,
        ],
    )
    .await?;
    let spend = |team: &str, id: &str, cost: f64| {
        serde_json::from_value::<BTreeMap<String, Value>>(json!({
            "request_id":id,"team_id":team,"api_key":"key-a","trace_id":"trace-a","response_id":"response-a",
            "spend":cost,"start_time":now_ms,"end_time":now_ms
        }))
    };
    insert_rows(
        &database.client,
        &writer,
        "eval_traces",
        InsertTable::SpendLogs,
        vec![
            spend("", "request-default", 0.25)?,
            spend("team-a", "request-a", 0.25)?,
            spend("team-b", "request-b", 99.0)?,
        ],
    )
    .await?;
    for statement in [
        "CREATE USER eval_reader",
        "GRANT SELECT ON eval_traces.* TO eval_reader",
        "REVOKE SELECT ON eval_traces.spend_logs FROM eval_reader",
    ] {
        database
            .client
            .post(writer.url().clone())
            .body(statement)
            .send()
            .await?
            .error_for_status()?;
    }
    let reader = Connection::configured(&database.url, "eval_traces", "eval_reader", "")?;
    Ok(Fixture {
        traces: EvalTraces::new(database.client.clone(), reader),
        database,
        now_ms,
    })
}

#[rstest]
#[case::standalone("", Some("private-default"))]
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
#[case::standalone_session("", "private-default", TraceAttribute::SessionId, "session-a")]
#[case::standalone_trace("", "private-default", TraceAttribute::TraceId, "trace-a")]
#[case::team_session("team-a", "private-a", TraceAttribute::SessionId, "session-a")]
#[case::team_trace("team-a", "private-a", TraceAttribute::TraceId, "trace-a")]
#[tokio::test]
async fn eval_reads_keep_spans_and_spend_within_the_authenticated_team_without_raw_access(
    #[future] populated: TestResult<Fixture>,
    #[case] team: &str,
    #[case] expected_input: &str,
    #[case] attribute: TraceAttribute,
    #[case] value: &str,
) -> TestResult {
    let fixture = populated.await?;
    let reader = Connection::configured(&fixture.database.url, "eval_traces", "eval_reader", "")?;
    assert!(
        litellm_traces_clickhouse::execute_read(
            &fixture.database.client,
            &reader,
            "SELECT count() FROM spend_logs",
            &BTreeMap::new(),
        )
        .await
        .is_err()
    );
    let trace = fixture
        .traces
        .read(
            team,
            &TraceRef {
                attribute,
                value: value.into(),
            },
            fixture.now_ms + 60_000,
        )
        .await?
        .unwrap();
    assert_eq!(trace.spans.len(), 2);
    assert_eq!(trace.spans[0].input, expected_input);
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
            "TraceId":"different-trace","SpanId":"root-two","TeamId":"team-a","ApiKeyHash":"key-a",
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
    assert!(
        trace
            .spans
            .iter()
            .any(|span| span.span_id.ends_with(":root-two"))
    );
    assert!(trace.root_ended_at_ms.is_some());
    assert_eq!(trace.agent, "agent");
    assert_eq!(trace.gateway_cost_usd, 0.25);
    Ok(())
}

#[rstest]
#[case::session(TraceAttribute::SessionId, "orphan-session")]
#[case::trace(TraceAttribute::TraceId, "orphan-trace")]
#[tokio::test]
async fn child_only_traces_can_be_resolved_for_idle_closure(
    #[future] populated: TestResult<Fixture>,
    #[case] attribute: TraceAttribute,
    #[case] value: &str,
) -> TestResult {
    let fixture = populated.await?;
    let writer = Connection::writer(&fixture.database.url)?;
    insert_rows(&fixture.database.client, &writer, "eval_traces", InsertTable::OtelTraces,
        vec![serde_json::from_value(json!({"Timestamp":OffsetDateTime::now_utc().unix_timestamp_nanos() as i64,
            "TraceId":"orphan-trace","SpanId":"child","ParentSpanId":"missing-root","TeamId":"team-a","ApiKeyHash":"key-a",
            "ResourceAttributes":{"session.id":"orphan-session","agent.name":"agent","agent.version":"sha","deployment.environment":"lens-eval"}}))?]).await?;
    let trace = fixture
        .traces
        .read(
            "team-a",
            &TraceRef {
                attribute,
                value: value.into(),
            },
            fixture.now_ms + 180_000,
        )
        .await?
        .unwrap();
    assert_eq!(trace.spans.len(), 1);
    assert_eq!(trace.root_ended_at_ms, None);
    assert!(trace.last_received_at_ms >= fixture.now_ms);
    assert_eq!(trace.agent, "agent");
    assert_eq!(trace.version, "sha");
    assert_eq!(trace.environment, "lens-eval");
    Ok(())
}

#[rstest]
#[case::gateway_call("litellm_request:call-a", "litellm_call_id", "call-a")]
#[case::gateway_request("litellm_request:request-cost", "request_id", "request-cost")]
#[case::provider_request("provider_request:provider-a", "provider_request_id", "provider-a")]
#[case::provider_response("provider_response:response-cost", "response_id", "response-cost")]
#[tokio::test]
async fn typed_call_keys_match_spend_without_a_matching_trace_id(
    #[future] populated: TestResult<Fixture>,
    #[case] key: &str,
    #[case] spend_field: &str,
    #[case] spend_value: &str,
    #[values("", "gateway-trace")] spend_trace: &str,
) -> TestResult {
    let fixture = populated.await?;
    let writer = Connection::writer(&fixture.database.url)?;
    let timestamp = OffsetDateTime::now_utc().unix_timestamp_nanos() as i64;
    let spans = ["root", "duplicate-key"]
        .into_iter()
        .map(|span| {
            serde_json::from_value(json!({
                "Timestamp":timestamp,"TraceId":"typed-trace","SpanId":span,
                "ParentSpanId":if span == "root" { "" } else { "root" },
                "TeamId":"team-a","ApiKeyHash":"key-a","CallKeys":[key],"LiteLLMRequestId":""
            }))
        })
        .collect::<Result<Vec<_>, _>>()?;
    insert_rows(
        &fixture.database.client,
        &writer,
        "eval_traces",
        InsertTable::OtelTraces,
        spans,
    )
    .await?;
    let spend = [("team-a", 0.75), ("team-b", 99.0)]
        .into_iter()
        .map(|(team, cost)| {
            let mut row = json!({
                "request_id":"request-cost","team_id":team,"api_key":"key-a","trace_id":spend_trace,
                "spend":cost,"start_time":fixture.now_ms,"end_time":fixture.now_ms
            });
            row[spend_field] = json!(spend_value);
            serde_json::from_value(row)
        })
        .collect::<Result<Vec<_>, _>>()?;
    insert_rows(
        &fixture.database.client,
        &writer,
        "eval_traces",
        InsertTable::SpendLogs,
        spend,
    )
    .await?;
    let trace = fixture
        .traces
        .read(
            "team-a",
            &TraceRef {
                attribute: TraceAttribute::TraceId,
                value: "typed-trace".into(),
            },
            fixture.now_ms + 60_000,
        )
        .await?
        .unwrap();
    assert_eq!(trace.spans.len(), 2);
    assert_eq!(trace.gateway_cost_usd, 0.75);
    Ok(())
}

#[rstest]
#[case::transport("transport:")]
#[case::gateway_attempt("gateway_attempt:")]
#[tokio::test]
async fn transport_call_keys_use_the_original_trace_for_spend(
    #[future] populated: TestResult<Fixture>,
    #[case] key: &str,
) -> TestResult {
    let fixture = populated.await?;
    let writer = Connection::writer(&fixture.database.url)?;
    insert_rows(
        &fixture.database.client,
        &writer,
        "eval_traces",
        InsertTable::OtelTraces,
        vec![serde_json::from_value(json!({
            "Timestamp":OffsetDateTime::now_utc().unix_timestamp_nanos() as i64,
            "TraceId":"rewritten-trace","SpanId":"root","TeamId":"team-a","ApiKeyHash":"key-a",
            "CallKeys":[key],"SpanAttributes":{"lens.original_trace_id":"trace-a"}
        }))?],
    )
    .await?;
    let trace = fixture
        .traces
        .read(
            "team-a",
            &TraceRef {
                attribute: TraceAttribute::TraceId,
                value: "rewritten-trace".into(),
            },
            fixture.now_ms + 60_000,
        )
        .await?
        .unwrap();
    assert_eq!(trace.gateway_cost_usd, 0.25);
    Ok(())
}

#[rstest]
#[case::session(TraceAttribute::SessionId, "session-a")]
#[case::trace(TraceAttribute::TraceId, "trace-a")]
#[tokio::test]
async fn credential_collisions_never_combine_trace_bodies_or_source_metadata(
    #[future] populated: TestResult<Fixture>,
    #[case] attribute: TraceAttribute,
    #[case] value: &str,
    #[values("", "team-a")] team: &str,
) -> TestResult {
    let fixture = populated.await?;
    let writer = Connection::writer(&fixture.database.url)?;
    insert_rows(
        &fixture.database.client,
        &writer,
        "eval_traces",
        InsertTable::OtelTraces,
        vec![serde_json::from_value(json!({
            "Timestamp":OffsetDateTime::now_utc().unix_timestamp_nanos() as i64,
            "TraceId":"trace-a","SpanId":"collision","TeamId":team,"ApiKeyHash":"different-key",
            "Input":"different credential's private trace", "SpanAttributes":{"session.id":"session-a","repo_url":"private-repository"}
        }))?],
    ).await?;
    assert!(matches!(
        fixture
            .traces
            .read(
                team,
                &TraceRef {
                    attribute,
                    value: value.into()
                },
                fixture.now_ms + 60_000
            )
            .await,
        Err(litellm_traces_clickhouse::Error::InvalidResponse)
    ));
    assert!(matches!(
        fixture.traces.root_attributes(team, "trace-a").await,
        Err(litellm_traces_clickhouse::Error::InvalidResponse)
    ));
    Ok(())
}

#[rstest]
#[case::matching_key("key-a", "", 0.75)]
#[case::matching_user("other-key", "owner-a", 0.75)]
#[case::different_owner("other-key", "other-user", 0.25)]
#[tokio::test]
async fn spend_matches_require_trace_ownership_within_the_same_team(
    #[future] populated: TestResult<Fixture>,
    #[case] key: &str,
    #[case] user: &str,
    #[case] expected: f64,
    #[values("", "team-a")] team: &str,
) -> TestResult {
    let fixture = populated.await?;
    let writer = Connection::writer(&fixture.database.url)?;
    insert_rows(
        &fixture.database.client,
        &writer,
        "eval_traces",
        InsertTable::SpendLogs,
        vec![serde_json::from_value(json!({
            "request_id":"same-team-request","team_id":team,"api_key":key,"user":user,
            "trace_id":"trace-a","response_id":"response-a","spend":0.5,
            "start_time":fixture.now_ms,"end_time":fixture.now_ms
        }))?],
    )
    .await?;
    let trace = fixture
        .traces
        .read(
            team,
            &TraceRef {
                attribute: TraceAttribute::TraceId,
                value: "trace-a".into(),
            },
            fixture.now_ms + 60_000,
        )
        .await?
        .unwrap();
    assert_eq!(trace.gateway_cost_usd, expected);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn session_graph_keeps_reused_span_ids_in_their_own_traces(
    #[future] populated: TestResult<Fixture>,
) -> TestResult {
    let fixture = populated.await?;
    let writer = Connection::writer(&fixture.database.url)?;
    let span = |id: &str, parent: &str| {
        serde_json::from_value(json!({
            "Timestamp":OffsetDateTime::now_utc().unix_timestamp_nanos() as i64,
            "TraceId":"trace-b","SpanId":id,"ParentSpanId":parent,
            "TeamId":"team-a","ApiKeyHash":"key-a","SpanName":"second trace",
            "SpanAttributes":{"session.id":"session-a"}
        }))
    };
    let rows = vec![span("root", "0000000000000000")?, span("tool", "root")?];
    insert_rows(
        &fixture.database.client,
        &writer,
        "eval_traces",
        InsertTable::OtelTraces,
        rows,
    )
    .await?;
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
    let ids: std::collections::BTreeSet<_> = trace.spans.iter().map(|span| &span.span_id).collect();
    assert_eq!(ids.len(), 4);
    assert_eq!(
        trace
            .spans
            .iter()
            .filter(|span| span.parent_span_id.is_empty())
            .count(),
        2
    );
    let child = trace
        .spans
        .iter()
        .find(|span| span.name == "second trace" && !span.parent_span_id.is_empty())
        .unwrap();
    let root = trace
        .spans
        .iter()
        .find(|span| span.span_id == child.parent_span_id)
        .unwrap();
    assert_eq!(root.name, "second trace");
    Ok(())
}

#[fixture]
fn paged_spans() -> TestResult<Vec<BTreeMap<String, Value>>> {
    let timestamp = OffsetDateTime::now_utc().unix_timestamp_nanos() as i64;
    (0..1025)
        .map(|index| {
            Ok(serde_json::from_value(json!({
                "Timestamp":timestamp,"EngineReceivedMs":1,"TraceId":"paged-trace",
                "SpanId":format!("{index:04}"),"ParentSpanId":if index == 0 { "" } else { "0000" },
                "TeamId":"team-a","ApiKeyHash":"key-a","Output":"original",
                "ResourceAttributes":{"session.id":"paged-session","agent.version":"sha"}
            }))?)
        })
        .collect()
}

#[rstest]
#[tokio::test]
async fn paged_reads_keep_every_span_once_and_use_the_latest_receipt(
    #[future] populated: TestResult<Fixture>,
    paged_spans: TestResult<Vec<BTreeMap<String, Value>>>,
) -> TestResult {
    let fixture = populated.await?;
    let writer = Connection::writer(&fixture.database.url)?;
    let mut rows = paged_spans?;
    let mut replacement = rows[0].clone();
    replacement.insert("EngineReceivedMs".into(), json!(2));
    replacement.insert("Output".into(), json!("updated"));
    rows.push(replacement);
    insert_rows(
        &fixture.database.client,
        &writer,
        "eval_traces",
        InsertTable::OtelTraces,
        rows,
    )
    .await?;
    let trace = fixture
        .traces
        .read(
            "team-a",
            &TraceRef {
                attribute: TraceAttribute::SessionId,
                value: "paged-session".into(),
            },
            fixture.now_ms + 60_000,
        )
        .await?
        .unwrap();
    assert_eq!(trace.spans.len(), 1025);
    assert_eq!(
        trace
            .spans
            .iter()
            .map(|span| &span.span_id)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        1025
    );
    let root = trace
        .spans
        .iter()
        .find(|span| span.parent_span_id.is_empty())
        .unwrap();
    assert_eq!(root.output, "updated");
    assert_eq!(trace.version, "sha");
    Ok(())
}
