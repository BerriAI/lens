use std::collections::BTreeMap;

use litellm_traces_clickhouse::{
    Connection, InsertTable, Parameter, ReadQuery, ensure_schema, execute_named_read, insert_rows,
};
use rstest::{fixture, rstest};
use serde_json::{Value, json};

mod support;

use support::{ClickHouseDatabase, TestResult, database};

const DATABASE: &str = "trace_test";
const TEAM: &str = "team-a";
const KEY: &str = "key-a";
const PROD: Trace = Trace {
    id: "prod-trace",
    root: "prod-root",
    agent: "prod-agent",
};
const EVAL: Trace = Trace {
    id: "eval-trace",
    root: "eval-root",
    agent: "eval-agent",
};

#[derive(Clone, Copy)]
struct Trace {
    id: &'static str,
    root: &'static str,
    agent: &'static str,
}

#[derive(Clone, Copy)]
enum ReadPath {
    Sample,
    Content,
    Evidence,
    Agents,
    ListTraces,
    TraceAgents,
}

struct Seeded {
    database: ClickHouseDatabase,
    start_ms: i64,
}

fn span(trace: Trace, span_id: &str, parent: &str, environment: &str, at_ms: i64) -> Value {
    let resource = if environment.is_empty() {
        json!({"service.name": "agent"})
    } else {
        json!({"service.name": "agent", "deployment.environment": environment})
    };
    json!({
        "Timestamp": at_ms * 1_000_000, "Duration": 1_000_000,
        "TraceId": trace.id, "SpanId": span_id, "ParentSpanId": parent,
        "SpanName": span_id, "ServiceName": "agent", "ResourceAttributes": resource,
        "TeamId": TEAM, "ApiKeyHash": KEY, "ObservationType": "agent",
        "AgentName": trace.agent, "Input": format!("hello from {}", trace.id),
    })
}

#[fixture]
async fn seeded(#[future(awt)] database: TestResult<ClickHouseDatabase>) -> TestResult<Seeded> {
    let database = database?;
    let writer = Connection::writer(&database.url)?;
    ensure_schema(&database.client, &writer, DATABASE, 7).await?;
    let start_ms = (time::OffsetDateTime::now_utc().unix_timestamp() - 3_600) * 1_000;
    let rows = [
        span(PROD, PROD.root, "", "production", start_ms + 1_000),
        span(PROD, "prod-child", PROD.root, "", start_ms + 2_000),
        span(EVAL, EVAL.root, "", "lens-eval", start_ms + 1_000),
        span(EVAL, "eval-child", EVAL.root, "", start_ms + 2_000),
    ]
    .into_iter()
    .map(serde_json::from_value)
    .collect::<Result<Vec<BTreeMap<String, Value>>, _>>()?;
    insert_rows(
        &database.client,
        &writer,
        DATABASE,
        InsertTable::OtelTraces,
        rows,
    )
    .await?;
    Ok(Seeded { database, start_ms })
}

fn parameters(path: ReadPath, trace: Trace, start_ms: i64) -> TestResult<BTreeMap<String, Value>> {
    let end_ms = start_ms + 7_200_000;
    let lens_access = json!({"all_teams": 0, "team": TEAM, "key_hash": KEY});
    let read_access = json!({"all_teams": 0, "user_id": "", "team_ids": [TEAM]});
    let content = json!({
        "source": "traces", "id": trace.id, "record_team": TEAM, "trace_ref": "",
        "start_time": time::OffsetDateTime::from_unix_timestamp(start_ms / 1_000)?
            .format(&time::format_description::well_known::Rfc3339)?,
    });
    let specific = match path {
        ReadPath::Sample => json!({
            "source": "traces", "start": start_ms, "end": end_ms, "agent_name": "", "service": "",
            "filter_keys": [], "filter_values": [], "selected_team": "", "execution_ids": [],
            "sample_cap": 0, "sample_percent": 100, "preview": 0, "after": "", "limit": 100, "offset": 0,
        }),
        ReadPath::Content => merge(&content, &json!({"cursor": "", "offset": 1})),
        ReadPath::Evidence => merge(
            &content,
            &json!({"span": trace.root, "quote": format!("hello from {}", trace.id)}),
        ),
        ReadPath::Agents => json!({}),
        ReadPath::ListTraces => json!({
            "start_ms": start_ms, "end_ms": end_ms, "cursor_ms": 0, "cursor_trace_id": "", "limit": 100,
        }),
        ReadPath::TraceAgents => json!({"start_ms": start_ms, "end_ms": end_ms, "limit": 100}),
    };
    let access = match path {
        ReadPath::ListTraces | ReadPath::TraceAgents => read_access,
        ReadPath::Sample | ReadPath::Content | ReadPath::Evidence | ReadPath::Agents => lens_access,
    };
    Ok(serde_json::from_value(merge(&access, &specific))?)
}

fn merge(base: &Value, extra: &Value) -> Value {
    let fields = base
        .as_object()
        .into_iter()
        .chain(extra.as_object())
        .flatten()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    Value::Object(fields)
}

fn query(path: ReadPath) -> ReadQuery {
    match path {
        ReadPath::Sample => ReadQuery::Sample,
        ReadPath::Content => ReadQuery::Content,
        ReadPath::Evidence => ReadQuery::Evidence,
        ReadPath::Agents => ReadQuery::Agents,
        ReadPath::ListTraces => ReadQuery::ListTraces,
        ReadPath::TraceAgents => ReadQuery::TraceAgents,
    }
}

fn shows(path: ReadPath, trace: Trace, rows: &[Value]) -> bool {
    match path {
        ReadPath::Sample | ReadPath::ListTraces => {
            rows.iter().any(|row| row["trace_id"] == trace.id)
        }
        ReadPath::Agents | ReadPath::TraceAgents => {
            rows.iter().any(|row| row["agent_name"] == trace.agent)
        }
        ReadPath::Content => !rows.is_empty(),
        ReadPath::Evidence => rows.first().is_some_and(|row| {
            row["count"].as_u64().unwrap_or(0) > 0
                || row["count"].as_str().is_some_and(|count| count != "0")
        }),
    }
}

async fn visible(seeded: &Seeded, path: ReadPath, trace: Trace) -> TestResult<bool> {
    let reader = Connection::reader(&seeded.database.url, DATABASE)?;
    let parameters: BTreeMap<String, Parameter> = serde_json::from_value(serde_json::to_value(
        parameters(path, trace, seeded.start_ms)?,
    )?)?;
    let body: Value = serde_json::from_str(
        &execute_named_read(&seeded.database.client, &reader, query(path), &parameters).await?,
    )?;
    let rows = body["data"].as_array().ok_or("no data")?;
    Ok(shows(path, trace, rows))
}

#[rstest]
#[case::sample(ReadPath::Sample)]
#[case::content(ReadPath::Content)]
#[case::evidence(ReadPath::Evidence)]
#[case::agents(ReadPath::Agents)]
#[case::list_traces(ReadPath::ListTraces)]
#[case::trace_agents(ReadPath::TraceAgents)]
#[tokio::test]
async fn eval_traces_never_reach_findings_or_dashboard_reads(
    #[future(awt)] seeded: TestResult<Seeded>,
    #[case] path: ReadPath,
) -> TestResult {
    let seeded = seeded?;
    assert!(visible(&seeded, path, PROD).await?);
    assert!(!visible(&seeded, path, EVAL).await?);
    Ok(())
}
