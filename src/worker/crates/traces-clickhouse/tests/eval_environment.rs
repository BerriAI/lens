use std::collections::BTreeMap;

use litellm_traces_clickhouse::{
    Connection, InsertTable, Parameter, QueryReaders, QueryScope, ReadQuery, TraceTable,
    ensure_schema, execute_named_read, insert_rows,
    query::lens::{LensAvailabilityRow, LensEvidenceRow},
    query_sql,
};
use rstest::{fixture, rstest};
use serde::Deserialize;
use serde_json::{Value, json};

mod support;

use support::{ClickHouseDatabase, TestResult, database};

const DATABASE: &str = "trace_test";
const KEY: &str = "key-a";
const PROD: Trace = Trace {
    team: "team-a",
    id: "prod-trace",
    child: "prod-child",
    agent: "prod-agent",
    request: "prod-request",
};
const EVAL: Trace = Trace {
    team: "team-a",
    id: "eval-trace",
    child: "eval-child",
    agent: "eval-agent",
    request: "eval-request",
};
const EVAL_ONLY: Trace = Trace {
    team: "team-eval",
    id: "eval-only-trace",
    child: "eval-only-child",
    agent: "eval-only-agent",
    request: "eval-only-request",
};

#[derive(Clone, Copy)]
struct Trace {
    team: &'static str,
    id: &'static str,
    child: &'static str,
    agent: &'static str,
    request: &'static str,
}

#[derive(Clone, Copy, Debug)]
enum Source {
    Traces,
    Requests,
}

impl Source {
    fn name(self) -> &'static str {
        match self {
            Self::Traces => "traces",
            Self::Requests => "requests",
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum ReadPath {
    Named(ReadQuery, Source),
    RawSql(TraceTable),
}

#[derive(Deserialize)]
struct Rows<T> {
    data: Vec<T>,
}

#[derive(Deserialize)]
struct Matched {
    matched: u8,
}

fn exempt(query: ReadQuery) -> Box<dyn std::error::Error> {
    format!("{query} opens one record by id, so the list that linked to it already filtered it")
        .into()
}

struct Seeded {
    database: ClickHouseDatabase,
    start_ms: i64,
}

fn span(trace: Trace, span: &str, parent: &str, environment: Option<&str>, at_ms: i64) -> Value {
    let resource = match environment {
        Some(environment) => {
            json!({"service.name": "agent", "deployment.environment": environment})
        }
        None => json!({"service.name": "agent"}),
    };
    json!({
        "Timestamp": at_ms * 1_000_000, "Duration": 1_000_000,
        "TraceId": trace.id, "SpanId": span, "ParentSpanId": parent,
        "SpanName": span, "ServiceName": "agent", "ResourceAttributes": resource,
        "TeamId": trace.team, "ApiKeyHash": KEY, "ObservationType": "agent",
        "AgentName": trace.agent, "Input": format!("hello from {span}"),
        "LiteLLMRequestId": if parent.is_empty() { trace.request } else { "" },
    })
}

fn request(trace: Trace, at_ms: i64) -> Value {
    json!({
        "request_id": trace.request, "response_id": trace.request, "team_id": trace.team, "api_key": KEY,
        "model": "model", "start_time": at_ms, "end_time": at_ms + 1, "metadata": "{}",
        "messages": format!("hello from {}", trace.request),
    })
}

fn rows(values: Vec<Value>) -> TestResult<Vec<BTreeMap<String, Value>>> {
    values
        .into_iter()
        .map(|value| serde_json::from_value(value).map_err(Into::into))
        .collect()
}

#[fixture]
async fn seeded(#[future(awt)] database: TestResult<ClickHouseDatabase>) -> TestResult<Seeded> {
    let database = database?;
    let writer = Connection::writer(&database.url)?;
    ensure_schema(&database.client, &writer, DATABASE, 7).await?;
    let start_ms = (time::OffsetDateTime::now_utc().unix_timestamp() - 3_600) * 1_000;
    let spans = rows(vec![
        span(PROD, "prod-root", "", Some("production"), start_ms + 1_000),
        span(PROD, PROD.child, "prod-root", None, start_ms + 2_000),
        span(EVAL, "eval-root", "", Some("lens-eval"), start_ms + 1_000),
        span(EVAL, EVAL.child, "eval-root", None, start_ms + 2_000),
        span(
            EVAL_ONLY,
            "eval-only-root",
            "",
            Some("lens-eval"),
            start_ms + 1_000,
        ),
        span(
            EVAL_ONLY,
            EVAL_ONLY.child,
            "eval-only-root",
            None,
            start_ms + 2_000,
        ),
    ])?;
    insert_rows(
        &database.client,
        &writer,
        DATABASE,
        InsertTable::OtelTraces,
        spans,
    )
    .await?;
    let requests = rows(vec![
        request(PROD, start_ms + 2_000),
        request(EVAL, start_ms + 2_000),
        request(EVAL_ONLY, start_ms + 2_000),
    ])?;
    insert_rows(
        &database.client,
        &writer,
        DATABASE,
        InsertTable::SpendLogs,
        requests,
    )
    .await?;
    Ok(Seeded { database, start_ms })
}

fn text(value: &str) -> Parameter {
    Parameter::Text(value.to_owned())
}

fn strings(values: &[&str]) -> Parameter {
    Parameter::Strings(values.iter().map(|value| (*value).to_owned()).collect())
}

fn parameters(
    query: ReadQuery,
    source: Source,
    trace: Trace,
    start_ms: i64,
) -> TestResult<BTreeMap<String, Parameter>> {
    let end_ms = start_ms + 7_200_000;
    let lens_access = [
        ("all_teams", Parameter::Unsigned(0)),
        ("team", text(trace.team)),
        ("key_hash", text(KEY)),
    ];
    let read_access = [
        ("all_teams", Parameter::Unsigned(0)),
        ("user_id", text("")),
        ("team_ids", strings(&[trace.team])),
    ];
    let start_time = time::OffsetDateTime::from_unix_timestamp(start_ms / 1_000)?
        .format(&time::format_description::well_known::Rfc3339)?;
    let (record, span) = match source {
        Source::Traces => (trace.id, trace.child),
        Source::Requests => (trace.request, trace.request),
    };
    let record = [
        ("source", text(source.name())),
        ("id", text(record)),
        ("record_team", text(trace.team)),
        ("trace_ref", text("")),
        ("start_time", text(&start_time)),
    ];
    let window = [
        ("start_ms", Parameter::Integer(start_ms)),
        ("end_ms", Parameter::Integer(end_ms)),
        ("limit", Parameter::Unsigned(100)),
    ];
    let fields: Vec<(&str, Parameter)> = match query {
        ReadQuery::Sample => lens_access
            .into_iter()
            .chain([
                ("source", text(source.name())),
                ("start", Parameter::Integer(start_ms)),
                ("end", Parameter::Integer(end_ms)),
                ("agent_name", text("")),
                ("service", text("")),
                ("filter_keys", strings(&[])),
                ("filter_values", strings(&[])),
                ("selected_team", text("")),
                ("execution_ids", strings(&[])),
                ("sample_cap", Parameter::Unsigned(0)),
                ("sample_percent", Parameter::Unsigned(100)),
                ("preview", Parameter::Unsigned(0)),
                ("after", text("")),
                ("limit", Parameter::Unsigned(100)),
                ("offset", Parameter::Unsigned(0)),
            ])
            .collect(),
        ReadQuery::Content => lens_access
            .into_iter()
            .chain(record)
            .chain([("cursor", text("")), ("offset", Parameter::Unsigned(1))])
            .collect(),
        ReadQuery::Evidence => lens_access
            .into_iter()
            .chain(record)
            .chain([
                ("span", text(span)),
                ("quote", text(&format!("hello from {span}"))),
            ])
            .collect(),
        ReadQuery::Agents | ReadQuery::Availability => lens_access.into_iter().collect(),
        ReadQuery::ListTraces => read_access
            .into_iter()
            .chain(window)
            .chain([
                ("cursor_ms", Parameter::Integer(0)),
                ("cursor_trace_id", text("")),
            ])
            .collect(),
        ReadQuery::TraceAgents => read_access.into_iter().chain(window).collect(),
        ReadQuery::TraceSpans
        | ReadQuery::TracePageSpans
        | ReadQuery::TraceIdentity
        | ReadQuery::SpanDetail
        | ReadQuery::SpanError
        | ReadQuery::SpendByResponseIds
        | ReadQuery::FeedbackTarget
        | ReadQuery::Feedback
        | ReadQuery::FeedbackSummary => return Err(exempt(query)),
    };
    Ok(fields
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect())
}

fn shows(query: ReadQuery, source: Source, trace: Trace, body: &str) -> TestResult<bool> {
    let rows: Vec<Value> = serde_json::from_str::<Rows<Value>>(body)?.data;
    let id = match source {
        Source::Traces => trace.id,
        Source::Requests => trace.request,
    };
    Ok(match query {
        ReadQuery::Sample | ReadQuery::ListTraces => rows.iter().any(|row| row["trace_id"] == id),
        ReadQuery::Agents | ReadQuery::TraceAgents => {
            rows.iter().any(|row| row["agent_name"] == trace.agent)
        }
        ReadQuery::Content => !rows.is_empty(),
        ReadQuery::Evidence => {
            serde_json::from_str::<Rows<LensEvidenceRow>>(body)?.data[0].count > 0
        }
        ReadQuery::Availability => {
            let row = &serde_json::from_str::<Rows<LensAvailabilityRow>>(body)?.data[0];
            match source {
                Source::Traces => row.traces == 1,
                Source::Requests => row.requests == 1,
            }
        }
        ReadQuery::TraceSpans
        | ReadQuery::TracePageSpans
        | ReadQuery::TraceIdentity
        | ReadQuery::SpanDetail
        | ReadQuery::SpanError
        | ReadQuery::SpendByResponseIds
        | ReadQuery::FeedbackTarget
        | ReadQuery::Feedback
        | ReadQuery::FeedbackSummary => return Err(exempt(query)),
    })
}

async fn raw_sql_matches(seeded: &Seeded, team: &str, sql: &str) -> TestResult<bool> {
    let client = &seeded.database.client;
    let scope = QueryScope::Owned {
        user_id: String::new(),
        team_ids: vec![team.to_owned()],
    };
    let reader = QueryReaders::new(
        Connection::writer(&seeded.database.url)?,
        DATABASE.to_owned(),
    )
    .connection(client, &scope, "test-master-secret")
    .await?;
    let body = query_sql(client, &reader, sql).await?;
    Ok(serde_json::from_str::<Rows<Matched>>(&body)?.data[0].matched == 1)
}

async fn visible(seeded: &Seeded, path: ReadPath, trace: Trace) -> TestResult<bool> {
    let client = &seeded.database.client;
    match path {
        ReadPath::Named(query, source) => {
            let reader = Connection::reader(&seeded.database.url, DATABASE)?;
            let parameters = parameters(query, source, trace, seeded.start_ms)?;
            let body = execute_named_read(client, &reader, query, &parameters).await?;
            shows(query, source, trace, &body)
        }
        ReadPath::RawSql(table) => {
            let condition = match table {
                TraceTable::OtelTraces | TraceTable::AgentTracesByKey => {
                    format!("TraceId = '{}'", trace.id)
                }
                TraceTable::SpendLogs => format!("request_id = '{}'", trace.request),
            };
            let sql = format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE {condition}) AS matched");
            raw_sql_matches(seeded, trace.team, &sql).await
        }
    }
}

#[rstest]
#[case::availability_traces(ReadPath::Named(ReadQuery::Availability, Source::Traces), EVAL_ONLY)]
#[case::availability_requests(
    ReadPath::Named(ReadQuery::Availability, Source::Requests),
    EVAL_ONLY
)]
#[case::sample_traces(ReadPath::Named(ReadQuery::Sample, Source::Traces), EVAL)]
#[case::sample_requests(ReadPath::Named(ReadQuery::Sample, Source::Requests), EVAL)]
#[case::content_traces(ReadPath::Named(ReadQuery::Content, Source::Traces), EVAL)]
#[case::content_requests(ReadPath::Named(ReadQuery::Content, Source::Requests), EVAL)]
#[case::evidence_traces(ReadPath::Named(ReadQuery::Evidence, Source::Traces), EVAL)]
#[case::evidence_requests(ReadPath::Named(ReadQuery::Evidence, Source::Requests), EVAL)]
#[case::agents(ReadPath::Named(ReadQuery::Agents, Source::Traces), EVAL)]
#[case::list_traces(ReadPath::Named(ReadQuery::ListTraces, Source::Traces), EVAL)]
#[case::trace_agents(ReadPath::Named(ReadQuery::TraceAgents, Source::Traces), EVAL)]
#[case::raw_sql_spans(ReadPath::RawSql(TraceTable::OtelTraces), EVAL)]
#[case::raw_sql_rollup(ReadPath::RawSql(TraceTable::AgentTracesByKey), EVAL)]
#[case::raw_sql_requests(ReadPath::RawSql(TraceTable::SpendLogs), EVAL)]
#[tokio::test]
async fn eval_traces_never_reach_findings_or_dashboard_reads(
    #[future(awt)] seeded: TestResult<Seeded>,
    #[case] path: ReadPath,
    #[case] eval: Trace,
) -> TestResult {
    let seeded = seeded?;
    assert!(visible(&seeded, path, PROD).await?, "{path:?} hides prod");
    assert!(!visible(&seeded, path, eval).await?, "{path:?} shows eval");
    Ok(())
}

#[rstest]
#[case::own_team("team-a", true)]
#[case::other_team("team-eval", false)]
#[tokio::test]
async fn raw_sql_readers_only_see_their_own_eval_trace_ids(
    #[future(awt)] seeded: TestResult<Seeded>,
    #[case] team: &str,
    #[case] expected: bool,
) -> TestResult {
    let seeded = seeded?;
    let sql = format!(
        "SELECT EXISTS(SELECT 1 FROM lens_eval_traces WHERE TraceId = '{}') AS matched",
        EVAL.id
    );
    assert_eq!(raw_sql_matches(&seeded, team, &sql).await?, expected);
    Ok(())
}
