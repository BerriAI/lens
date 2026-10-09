use std::collections::BTreeMap;

use litellm_storage_clickhouse::fetch;
use litellm_traces::query::named as contracts;
use litellm_traces_clickhouse::{
    Connection, InsertTable, Parameter, QueryScope, ReadQuery, execute_named_read, execute_read,
    insert_rows,
    query::named::{ListTraces, ListTracesParams, TraceSpans, TraceSpansParams},
    query_help, query_sql,
};
use rstest::{fixture, rstest};
use serde::Deserialize;
use serde_json::Value;

#[path = "queries/support.rs"]
mod fixtures;
mod support;

use fixtures::{SeededDatabase, insert_export, migrated_database, seeded_database};
use support::TestResult;

#[rstest]
#[tokio::test]
async fn lens_sample_keeps_spans_before_window_start_and_excludes_old_only_traces(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
) -> TestResult {
    let fixture = migrated_database?;
    let start_ms = time::OffsetDateTime::now_utc().unix_timestamp() * 1000 - 86_400_000;
    let end_ms = start_ms + 86_460_000;
    let rows = [
        (
            "late-root",
            "trace-with-slack",
            start_ms - 2 * 86_400_000,
            "",
        ),
        (
            "in-window",
            "trace-with-slack",
            start_ms + 1_000,
            "late-root",
        ),
        ("old-span", "trace-too-old", start_ms - 8 * 86_400_000, ""),
    ]
    .into_iter()
    .map(|(span_id, trace_id, timestamp_ms, parent_span_id)| {
        BTreeMap::from([
            (
                "Timestamp".into(),
                serde_json::json!(timestamp_ms * 1_000_000),
            ),
            ("Duration".into(), serde_json::json!(1_000_000)),
            ("TraceId".into(), serde_json::json!(trace_id)),
            ("SpanId".into(), serde_json::json!(span_id)),
            ("ParentSpanId".into(), serde_json::json!(parent_span_id)),
            ("SpanName".into(), serde_json::json!(span_id)),
            ("ObservationType".into(), serde_json::json!("agent")),
            ("TeamId".into(), serde_json::json!("team-lens")),
            ("ApiKeyHash".into(), serde_json::json!("")),
        ])
    })
    .collect();
    let writer = Connection::writer(&fixture.database.url)?;
    insert_rows(
        &fixture.database.client,
        &writer,
        fixtures::DATABASE,
        InsertTable::OtelTraces,
        rows,
    )
    .await?;
    let connection =
        Connection::configured(&fixture.database.url, fixtures::DATABASE, "default", "")?;
    let parameters = BTreeMap::from([
        ("source".into(), Parameter::Text("traces".into())),
        ("all_teams".into(), Parameter::Integer(0)),
        ("team".into(), Parameter::Text("team-lens".into())),
        ("key_hash".into(), Parameter::Text(String::new())),
        ("start".into(), Parameter::Unsigned(start_ms as u64)),
        ("end".into(), Parameter::Unsigned(end_ms as u64)),
        ("agent_name".into(), Parameter::Text(String::new())),
        ("service".into(), Parameter::Text(String::new())),
        ("filter_keys".into(), Parameter::Strings(Vec::new())),
        ("filter_values".into(), Parameter::Strings(Vec::new())),
        ("selected_team".into(), Parameter::Text(String::new())),
        ("execution_ids".into(), Parameter::Strings(Vec::new())),
        ("sample_cap".into(), Parameter::Unsigned(0)),
        ("sample_percent".into(), Parameter::Integer(100)),
        ("preview".into(), Parameter::Integer(0)),
        ("after".into(), Parameter::Text(String::new())),
        ("limit".into(), Parameter::Unsigned(10_000)),
        ("offset".into(), Parameter::Unsigned(0)),
    ]);
    let body = execute_named_read(
        &fixture.database.client,
        &connection,
        ReadQuery::Sample,
        &parameters,
    )
    .await?;
    let result: serde_json::Value = serde_json::from_str(&body)?;
    let executions = result["data"].as_array().ok_or("sample rows")?;
    let trace = executions
        .iter()
        .find(|row| row["trace_id"] == "trace-with-slack")
        .ok_or("sampled trace missing")?;
    let original_start = execute_read(
        &fixture.database.client,
        &connection,
        "SELECT toString(fromUnixTimestamp64Nano({timestamp:Int64})) AS start_time FORMAT JSON",
        &BTreeMap::from([(
            "timestamp".into(),
            Parameter::Integer((start_ms - 2 * 86_400_000) * 1_000_000),
        )]),
    )
    .await?;
    let original_start: serde_json::Value = serde_json::from_str(&original_start)?;
    assert_eq!(trace["span_count"].as_u64(), Some(2));
    assert_eq!(trace["start_time"], original_start["data"][0]["start_time"]);
    assert!(
        !executions
            .iter()
            .any(|row| row["trace_id"] == "trace-too-old")
    );
    Ok(())
}

#[derive(Clone, Copy, strum::AsRefStr)]
#[strum(serialize_all = "snake_case")]
enum ScopeCase {
    Admin,
    Team,
    OtherTeam,
}

impl ScopeCase {
    fn scope(self) -> QueryScope {
        match self {
            Self::Admin => QueryScope::All,
            Self::Team => QueryScope::Owned {
                user_id: String::new(),
                team_ids: vec!["team-a".into()],
            },
            Self::OtherTeam => QueryScope::Owned {
                user_id: String::new(),
                team_ids: vec!["team-b".into()],
            },
        }
    }
}

#[derive(Deserialize)]
struct QueryResult {
    data: Vec<Value>,
}

#[rstest]
#[case::costs(include_str!("queries/trace_costs.sql"), include_str!("queries/trace_costs.expected.json"))]
#[tokio::test]
async fn storage_queries_return_expected_rows(
    #[future(awt)] seeded_database: TestResult<SeededDatabase>,
    #[case] sql: &str,
    #[case] expected_json: &str,
    #[values(ScopeCase::Admin, ScopeCase::Team, ScopeCase::OtherTeam)] scope: ScopeCase,
) -> TestResult {
    let fixture = seeded_database?;
    let reader = fixture
        .readers
        .connection(&fixture.database.client, &scope.scope(), "fixture-secret")
        .await?;
    let result: QueryResult =
        serde_json::from_str(&query_sql(&fixture.database.client, &reader, sql).await?)?;
    let expected: BTreeMap<String, Vec<Value>> = serde_json::from_str(expected_json)?;
    assert_eq!(
        &result.data,
        expected
            .get(scope.as_ref())
            .ok_or("missing expected scope")?,
        "{}: {sql}",
        scope.as_ref()
    );
    Ok(())
}

#[rstest]
#[case::trace_summary("Trace summaries with tokens and errors", include_str!("../query/help/trace_summary.sql"), include_str!("queries/rollups.expected.json"))]
#[case::failed_spans("Recent failed spans", include_str!("../query/help/failed_spans.sql"), include_str!("queries/failed_spans.expected.json"))]
#[case::metadata_filter("Filter calls by nested metadata", include_str!("../query/help/metadata_filter.sql"), include_str!("queries/metadata_filters.expected.json"))]
#[tokio::test]
async fn documented_queries_render_and_return_expected_rows(
    #[future(awt)] seeded_database: TestResult<SeededDatabase>,
    fixture_clock: TestResult<u64>,
    #[case] name: &str,
    #[case] expected_sql: &str,
    #[case] expected_json: &str,
    #[values(ScopeCase::Admin, ScopeCase::Team, ScopeCase::OtherTeam)] scope: ScopeCase,
) -> TestResult {
    let fixture = seeded_database?;
    let reader = fixture
        .readers
        .connection(&fixture.database.client, &scope.scope(), "fixture-secret")
        .await?;
    let help = serde_json::to_value(query_help(&fixture.database.client, &reader).await?)?;
    let example = help["examples"]
        .as_array()
        .ok_or("missing examples")?
        .iter()
        .find(|example| example["name"] == name)
        .ok_or("missing documented query")?;
    let sql = example["sql"].as_str().ok_or("missing example SQL")?;
    assert_eq!(sql.trim(), expected_sql.trim());
    assert!(
        help["guide"]
            .as_str()
            .ok_or("missing guide")?
            .contains(&format!("{name}\n{sql}"))
    );
    let sql_at_fixture_time = sql.replace("now()", &format!("toDateTime({})", fixture_clock?));
    let result: QueryResult = serde_json::from_str(
        &query_sql(&fixture.database.client, &reader, &sql_at_fixture_time).await?,
    )?;
    let expected: BTreeMap<String, Vec<Value>> = serde_json::from_str(expected_json)?;
    assert_eq!(
        &result.data,
        expected
            .get(scope.as_ref())
            .ok_or("missing expected scope")?,
        "{}: {sql}",
        scope.as_ref()
    );
    Ok(())
}

#[fixture]
fn fixture_clock() -> TestResult<u64> {
    let spans = litellm_traces::decode_otlp(
        include_bytes!("../../traces/tests/fixtures/query_root.json"),
        Some("application/json"),
    )?;
    Ok(spans.first().ok_or("missing fixture root")?.start_ns / 1_000_000_000)
}

#[fixture]
fn admin_access() -> TestResult<contracts::ReadAccessParams> {
    Ok(serde_json::from_str(include_str!(
        "queries/read_access.json"
    ))?)
}

#[rstest]
#[tokio::test]
async fn typed_queries_read_normalized_spans_and_keep_trace_identities_separate(
    #[future(awt)] seeded_database: TestResult<SeededDatabase>,
    admin_access: TestResult<contracts::ReadAccessParams>,
) -> TestResult {
    let fixture = seeded_database?;
    let reader = fixture
        .readers
        .connection(&fixture.database.client, &QueryScope::All, "fixture-secret")
        .await?;
    let params = ListTracesParams::from(contracts::ListTracesParams {
        access: admin_access?,
        start_ms: 0,
        end_ms: i64::MAX / 1_000_000,
        cursor_ms: 0,
        cursor_trace_id: String::new(),
        limit: 10,
    });
    let traces = fetch::<ListTraces>(&fixture.database.client, &reader, &params).await?;
    assert_eq!(
        traces
            .iter()
            .map(|row| row.0.api_key_hash.as_str())
            .collect::<Vec<_>>(),
        ["key-b", "key-alt", "key-a"]
    );
    let trace = &traces[2].0;
    assert_eq!(
        (
            trace.span_count,
            trace.llm_calls,
            trace.tool_calls,
            trace.error_count
        ),
        (3, 1, 1, 1)
    );
    assert_eq!((trace.input_tokens, trace.output_tokens), (12, 6));
    assert_eq!(trace.input_preview, "Review the change");
    let span_params = TraceSpansParams {
        access: params.0.access,
        trace_id: trace.trace_id.clone(),
        trace_ref: trace.trace_ref.clone(),
    };
    let spans = fetch::<TraceSpans>(&fixture.database.client, &reader, &span_params).await?;
    assert_eq!(
        spans
            .iter()
            .map(|row| row.0.name.as_str())
            .collect::<Vec<_>>(),
        ["review", "completion", "lookup"]
    );
    assert!(
        spans
            .iter()
            .all(|row| row.0.api_key_hash == trace.api_key_hash)
    );
    assert_eq!(
        (
            spans[1].0.kind,
            spans[1].0.input_tokens,
            spans[1].0.output_tokens
        ),
        (litellm_traces::ObservationType::Llm, 12, 6)
    );
    assert_eq!(spans[2].0.status_message, "lookup timed out");
    Ok(())
}

#[rstest]
#[tokio::test]
async fn typed_trace_cursor_returns_the_next_fixture_trace(
    #[future(awt)] seeded_database: TestResult<SeededDatabase>,
    admin_access: TestResult<contracts::ReadAccessParams>,
) -> TestResult {
    let fixture = seeded_database?;
    let reader = fixture
        .readers
        .connection(&fixture.database.client, &QueryScope::All, "fixture-secret")
        .await?;
    let params = ListTracesParams::from(contracts::ListTracesParams {
        access: admin_access?,
        start_ms: 0,
        end_ms: i64::MAX / 1_000_000,
        cursor_ms: 0,
        cursor_trace_id: String::new(),
        limit: 1,
    });
    let first = fetch::<ListTraces>(&fixture.database.client, &reader, &params).await?;
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].0.api_key_hash, "key-b");
    let next_params = ListTracesParams::from(contracts::ListTracesParams {
        cursor_ms: first[0].0.start_ms,
        cursor_trace_id: first[0].0.trace_ref.clone(),
        ..params.0
    });
    let next = fetch::<ListTraces>(&fixture.database.client, &reader, &next_params).await?;
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].0.api_key_hash, "key-alt");
    assert_ne!(first[0].0.trace_ref, next[0].0.trace_ref);
    Ok(())
}

#[rstest]
#[case::billed_failure(include_bytes!("../../traces/tests/fixtures/google_adk_billed_failure.json"))]
#[case::retry(include_bytes!("../../traces/tests/fixtures/pydantic_ai_retry.json"))]
#[case::swarm(include_bytes!("../../traces/tests/fixtures/deepagents_swarm.json"))]
#[tokio::test]
async fn captured_sdk_exports_round_trip_through_clickhouse(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
    admin_access: TestResult<contracts::ReadAccessParams>,
    #[case] export: &[u8],
) -> TestResult {
    let fixture = migrated_database?;
    let decoded = insert_export(&fixture, export, "team-a", "key-a").await?;
    let reader = fixture
        .readers
        .connection(&fixture.database.client, &QueryScope::All, "fixture-secret")
        .await?;
    let params = TraceSpansParams {
        access: admin_access?,
        trace_id: decoded[0].trace_id.clone(),
        trace_ref: String::new(),
    };
    let stored = fetch::<TraceSpans>(&fixture.database.client, &reader, &params).await?;
    assert_eq!(stored.len(), decoded.len());
    let list_params = ListTracesParams::from(contracts::ListTracesParams {
        access: params.access,
        start_ms: 0,
        end_ms: i64::MAX / 1_000_000,
        cursor_ms: 0,
        cursor_trace_id: String::new(),
        limit: 10,
    });
    let traces = fetch::<ListTraces>(&fixture.database.client, &reader, &list_params).await?;
    assert_eq!(traces.len(), 1);
    let roots = decoded
        .iter()
        .filter(|span| span.parent_span_id.is_empty())
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), 1);
    assert_eq!(
        traces[0].0.status,
        serde_json::from_value::<litellm_traces::SpanStatus>(serde_json::json!(
            roots[0].status_code
        ))
        .unwrap()
    );
    assert_eq!(
        traces[0].0.error_count,
        decoded
            .iter()
            .filter(|span| span.status_code == "STATUS_CODE_ERROR")
            .count() as u64
    );
    let by_id: BTreeMap<_, _> = stored
        .iter()
        .map(|row| (row.0.span_id.as_str(), &row.0))
        .collect();
    for span in &decoded {
        let row = by_id
            .get(span.span_id.as_str())
            .ok_or("missing captured span")?;
        assert_eq!(row.parent_span_id, span.parent_span_id);
        assert_eq!(row.start_ns as u64, span.start_ns);
        assert_eq!(row.duration_ns, span.end_ns - span.start_ns);
        assert_eq!(row.input_tokens, span.normalized.input_tokens);
        assert_eq!(row.output_tokens, span.normalized.output_tokens);
        assert_eq!(
            row.status,
            serde_json::from_value::<litellm_traces::SpanStatus>(serde_json::json!(
                span.status_code
            ))
            .unwrap()
        );
    }
    Ok(())
}

#[rstest]
#[case::root_span("SpanAttributes")]
#[case::root_resource("ResourceAttributes")]
#[tokio::test]
async fn eval_traces_stay_out_of_production_sampling_and_dashboards(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
    #[case] attribute_field: &str,
) -> TestResult {
    use litellm_traces_clickhouse::query::lens::{
        ExecutionSource, LensAccessParams, LensAgents, LensAgentsParams, LensAvailability,
        LensAvailabilityParams, LensSample, LensSampleParams, TraceAgents, TraceAgentsParams,
    };
    let fixture = migrated_database?;
    let now_ms = time::OffsetDateTime::now_utc().unix_timestamp() * 1000;
    let start_ms = now_ms - 10_000;
    let end_ms = now_ms + 60_000;
    let trace_rows = [
        ("eval-trace", "eval-root", "", "eval-agent", true),
        (
            "eval-trace",
            "eval-child",
            "eval-root",
            "eval-child-agent",
            false,
        ),
        (
            "eval-trace",
            "eval-managed",
            "eval-root",
            "eval-agent",
            false,
        ),
        (
            "eval-trace",
            "eval-provider",
            "eval-root",
            "eval-agent",
            false,
        ),
        (
            "eval-trace",
            "eval-gateway",
            "eval-root",
            "eval-agent",
            false,
        ),
        (
            "eval-trace",
            "eval-legacy",
            "eval-root",
            "eval-agent",
            false,
        ),
        (
            "eval-trace",
            "eval-transport",
            "eval-root",
            "eval-agent",
            false,
        ),
        (
            "eval-trace",
            "eval-attempt",
            "eval-root",
            "eval-agent",
            false,
        ),
        (
            "production-trace",
            "production-root",
            "",
            "production-agent",
            false,
        ),
    ]
    .into_iter()
    .map(|(trace_id, span_id, parent, agent, eval)| {
        let mut row = BTreeMap::from([
            ("Timestamp".into(), serde_json::json!(start_ms * 1_000_000)),
            ("Duration".into(), serde_json::json!(1_000_000)),
            ("TraceId".into(), serde_json::json!(trace_id)),
            ("SpanId".into(), serde_json::json!(span_id)),
            ("ParentSpanId".into(), serde_json::json!(parent)),
            ("SpanName".into(), serde_json::json!(span_id)),
            ("ObservationType".into(), serde_json::json!("agent")),
            ("AgentName".into(), serde_json::json!(agent)),
            ("TeamId".into(), serde_json::json!("team-lens")),
            (
                "ApiKeyHash".into(),
                serde_json::json!(if trace_id == "production-trace" {
                    "key-production"
                } else {
                    "key-lens"
                }),
            ),
            (
                "LiteLLMRequestId".into(),
                serde_json::json!(if span_id == "eval-child" {
                    "eval-response"
                } else {
                    ""
                }),
            ),
            (
                "CallKeys".into(),
                serde_json::json!(match span_id {
                    "eval-managed" => vec![litellm_traces::CallKey::ProviderResponse(
                        "eval-managed-response".into()
                    )],
                    "eval-provider" => vec![litellm_traces::CallKey::ProviderRequest(
                        "eval-provider-request".into()
                    )],
                    "eval-gateway" => vec![litellm_traces::CallKey::LiteLlmRequest(
                        "eval-gateway-call".into()
                    )],
                    "eval-legacy" => vec![litellm_traces::CallKey::LiteLlmRequest(
                        "eval-legacy-request".into()
                    )],
                    "eval-transport" => vec![litellm_traces::CallKey::Transport],
                    "eval-attempt" => vec![litellm_traces::CallKey::GatewayAttempt],
                    _ => Vec::new(),
                }),
            ),
            (
                attribute_field.into(),
                if eval {
                    serde_json::json!({"deployment.environment":"lens-eval"})
                } else {
                    serde_json::json!({})
                },
            ),
        ]);
        if matches!(span_id, "eval-transport" | "eval-attempt") {
            row.entry("SpanAttributes".into())
                .or_insert_with(|| serde_json::json!({}))["lens.original_trace_id"] =
                serde_json::json!(format!("original-{span_id}"));
        }
        row
    })
    .collect();
    let writer = Connection::writer(&fixture.database.url)?;
    insert_rows(
        &fixture.database.client,
        &writer,
        fixtures::DATABASE,
        InsertTable::OtelTraces,
        trace_rows,
    )
    .await?;
    let spend_rows = [
        ("eval-by-trace", "eval-trace", "", serde_json::json!({})),
        (
            "eval-by-response",
            "",
            "eval-response",
            serde_json::json!({}),
        ),
        (
            "eval-by-managed-response",
            "",
            "resp_cmVzcG9uc2VfaWQ6ZXZhbC1tYW5hZ2VkLXJlc3BvbnNlO21vZGVsOmZpeHR1cmU=",
            serde_json::json!({}),
        ),
        ("eval-by-provider-request", "", "", serde_json::json!({})),
        ("eval-by-gateway-call", "", "", serde_json::json!({})),
        ("eval-legacy-request", "", "", serde_json::json!({})),
        (
            "eval-transport-request",
            "original-eval-transport",
            "",
            serde_json::json!({}),
        ),
        (
            "eval-attempt-request",
            "original-eval-attempt",
            "",
            serde_json::json!({}),
        ),
        (
            "eval-by-metadata",
            "",
            "",
            serde_json::json!({"requester_metadata":{"deployment.environment":"lens-eval"}}),
        ),
        (
            "production-request",
            "",
            "eval-response",
            serde_json::json!({}),
        ),
    ]
    .into_iter()
    .map(|(id, trace_id, response_id, metadata)| {
        BTreeMap::from([
            ("request_id".into(), serde_json::json!(id)),
            ("trace_id".into(), serde_json::json!(trace_id)),
            (
                "span_id".into(),
                serde_json::json!(match id {
                    "eval-transport-request" => "eval-transport",
                    "eval-attempt-request" => "eval-attempt",
                    _ => "",
                }),
            ),
            ("response_id".into(), serde_json::json!(response_id)),
            ("team_id".into(), serde_json::json!("team-lens")),
            (
                "api_key".into(),
                serde_json::json!(if id == "production-request" {
                    "key-production"
                } else {
                    "key-lens"
                }),
            ),
            (
                "provider_request_id".into(),
                serde_json::json!(if id == "eval-by-provider-request" {
                    "eval-provider-request"
                } else {
                    ""
                }),
            ),
            (
                "litellm_call_id".into(),
                serde_json::json!(if id == "eval-by-gateway-call" {
                    "eval-gateway-call"
                } else {
                    ""
                }),
            ),
            ("start_time".into(), serde_json::json!(start_ms)),
            ("end_time".into(), serde_json::json!(start_ms + 1)),
            ("metadata".into(), serde_json::json!(metadata.to_string())),
        ])
    })
    .collect();
    insert_rows(
        &fixture.database.client,
        &writer,
        fixtures::DATABASE,
        InsertTable::SpendLogs,
        spend_rows,
    )
    .await?;
    let connection =
        Connection::configured(&fixture.database.url, fixtures::DATABASE, "default", "")?;
    let availability = fetch::<LensAvailability>(
        &fixture.database.client,
        &connection,
        &LensAvailabilityParams {
            access: LensAccessParams {
                all_teams: false,
                team: "team-lens".into(),
                key_hash: "key-lens".into(),
            },
        },
    )
    .await?;
    assert_eq!(availability.len(), 1);
    assert_eq!(availability[0].traces, 0);
    assert_eq!(availability[0].requests, 0);
    let sampled = fetch::<LensSample>(
        &fixture.database.client,
        &connection,
        &LensSampleParams {
            access: LensAccessParams {
                all_teams: false,
                team: "team-lens".into(),
                key_hash: String::new(),
            },
            source: ExecutionSource::Both,
            start: start_ms as u64,
            end: end_ms as u64,
            agent_name: String::new(),
            service: String::new(),
            filter_keys: Vec::new(),
            filter_values: Vec::new(),
            selected_team: String::new(),
            execution_ids: Vec::new(),
            sample_cap: 0,
            sample_percent: 100.0,
            preview: 0,
            after: String::new(),
            limit: 100,
            offset: 0,
        },
    )
    .await?;
    let sampled_ids: std::collections::BTreeSet<_> =
        sampled.iter().map(|row| row.trace_id.as_str()).collect();
    assert_eq!(
        sampled_ids,
        std::collections::BTreeSet::from(["production-trace", "production-request"])
    );
    let access = contracts::ReadAccessParams {
        all_teams: false,
        user_id: String::new(),
        team_ids: vec!["team-lens".into()],
    };
    let listed = fetch::<ListTraces>(
        &fixture.database.client,
        &connection,
        &ListTracesParams(contracts::ListTracesParams {
            access: access.clone(),
            start_ms,
            end_ms,
            cursor_ms: 0,
            cursor_trace_id: String::new(),
            limit: 100,
        }),
    )
    .await?;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].0.trace_id, "production-trace");
    let agents = fetch::<TraceAgents>(
        &fixture.database.client,
        &connection,
        &TraceAgentsParams {
            all_teams: false,
            user_id: String::new(),
            team_ids: vec!["team-lens".into()],
            start_ms,
            end_ms,
            limit: 100,
        },
    )
    .await?;
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].agent_name, "production-agent");
    assert_eq!(agents[0].runs, 1);
    let discovered = fetch::<LensAgents>(
        &fixture.database.client,
        &connection,
        &LensAgentsParams {
            access: LensAccessParams {
                all_teams: false,
                team: "team-lens".into(),
                key_hash: String::new(),
            },
        },
    )
    .await?;
    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].agent_name, "production-agent");
    let explicit = fetch::<TraceSpans>(
        &fixture.database.client,
        &connection,
        &TraceSpansParams {
            access,
            trace_id: "eval-trace".into(),
            trace_ref: String::new(),
        },
    )
    .await?;
    assert_eq!(explicit.len(), 8);
    Ok(())
}
