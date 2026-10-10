use std::collections::BTreeMap;

use litellm_traces::query::named::{ReadAccessParams, TracePageSpansParams};
use litellm_traces_cache::TraceStore;
use litellm_traces_clickhouse::query::lens::{
    LensSampleEligibility, LensSampleParams, LensSignalSample,
};
use litellm_traces_clickhouse::{
    ClickHouseTraces, Connection, Parameter, ReadQuery, execute_named_read,
};
use rstest::rstest;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[path = "queries/support.rs"]
mod fixtures;
mod support;

use fixtures::{DATABASE, SeededDatabase, migrated_database};
use support::TestResult;

const SPANS_PER_DAY: u64 = 2_000;

async fn seed_days(fixture: &SeededDatabase, first_day: u64, days: u64) -> TestResult {
    seed_payload_days(fixture, first_day, days, 3000).await
}

async fn seed_payload_days(
    fixture: &SeededDatabase,
    first_day: u64,
    days: u64,
    payload_bytes: usize,
) -> TestResult {
    let count = SPANS_PER_DAY * days;
    let first_row = SPANS_PER_DAY * first_day;
    let query = format!(
        "INSERT INTO {DATABASE}.otel_traces \
         (Timestamp, TraceId, SpanId, ParentSpanId, SpanName, ServiceName, ObservationType, TeamId, ApiKeyHash, Duration, SpanAttributes) \
         SELECT now64(9) - toIntervalHour(intDiv(number, {SPANS_PER_DAY}) * 24 + 12 + {first_day} * 24), \
         if({first_day} = 0, concat('load-', toString(number + {first_row})), 'load-0'), \
         concat('span-', toString(number + {first_row})), \
         '', 'span', 'service', 'agent', 'load-team', '', 0, \
         if({first_day}=0 AND number < {SPANS_PER_DAY}, map('payload', repeat('x', {payload_bytes})), map()) \
         FROM numbers({count})"
    );
    fixture
        .database
        .client
        .post(&fixture.database.url)
        .body(query)
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn trace_start_time(fixture: &SeededDatabase) -> TestResult<String> {
    let query = format!(
        "SELECT toString(Timestamp, 'UTC') AS start_time FROM {DATABASE}.otel_traces \
         WHERE TraceId = 'load-0' LIMIT 1 FORMAT JSON"
    );
    let response = fixture
        .database
        .client
        .post(&fixture.database.url)
        .body(query)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let result: Value = serde_json::from_str(&response)?;
    result["data"][0]["start_time"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "trace start time missing".into())
}

fn content_parameters(start_time: &str) -> BTreeMap<String, Parameter> {
    BTreeMap::from([
        ("source".into(), Parameter::Text("traces".into())),
        ("all_teams".into(), Parameter::Integer(0)),
        ("team".into(), Parameter::Text("load-team".into())),
        ("key_hash".into(), Parameter::Text(String::new())),
        ("id".into(), Parameter::Text("load-0".into())),
        ("record_team".into(), Parameter::Text("load-team".into())),
        ("start_time".into(), Parameter::Text(start_time.into())),
        ("trace_ref".into(), Parameter::Text(String::new())),
        ("cursor".into(), Parameter::Text(String::new())),
        ("offset".into(), Parameter::Integer(1)),
    ])
}

async fn content(fixture: &SeededDatabase, start_time: &str, query_id: &str) -> TestResult {
    let connection = Connection::configured(
        &format!("{}?query_id={query_id}", fixture.database.url),
        DATABASE,
        "default",
        "",
    )?;
    let response = execute_named_read(
        &fixture.database.client,
        &connection,
        ReadQuery::Content,
        &content_parameters(start_time),
    )
    .await?;
    let result: Value = serde_json::from_str(&response)?;
    assert!(!result["data"].as_array().ok_or("content rows")?.is_empty());
    Ok(())
}

fn sample_parameters(start: u64, end: u64) -> BTreeMap<String, Parameter> {
    BTreeMap::from([
        ("source".into(), Parameter::Text("traces".into())),
        ("all_teams".into(), Parameter::Integer(0)),
        ("team".into(), Parameter::Text("load-team".into())),
        ("key_hash".into(), Parameter::Text(String::new())),
        ("start".into(), Parameter::Unsigned(start)),
        ("end".into(), Parameter::Unsigned(end)),
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
    ])
}

async fn sample(
    fixture: &SeededDatabase,
    start: u64,
    end: u64,
    query_id: &str,
) -> TestResult<(usize, usize)> {
    let mut url = Connection::configured(&fixture.database.url, DATABASE, "default", "")?
        .url()
        .clone();
    url.query_pairs_mut().append_pair("query_id", query_id);
    let connection = Connection::parse(url.as_str())?;
    let response = execute_named_read(
        &fixture.database.client,
        &connection,
        ReadQuery::Sample,
        &sample_parameters(start, end),
    )
    .await?;
    let result: Value = serde_json::from_str(&response)?;
    Ok((
        result["data"].as_array().ok_or("sample rows")?.len(),
        response.len(),
    ))
}

#[derive(serde::Deserialize)]
struct QueryStatistics {
    read_rows: u64,
    read_bytes: u64,
    query_duration_ms: u64,
}

async fn query_statistics(fixture: &SeededDatabase, query_id: &str) -> TestResult<QueryStatistics> {
    fixture
        .database
        .client
        .post(&fixture.database.url)
        .body("SYSTEM FLUSH LOGS")
        .send()
        .await?
        .error_for_status()?;
    let response = fixture
        .database
        .client
        .post(&fixture.database.url)
        .body(format!(
            "SELECT sum(read_rows) AS read_rows, sum(read_bytes) AS read_bytes, sum(query_duration_ms) AS query_duration_ms \
             FROM system.query_log WHERE type = 'QueryFinish' AND query_id = '{query_id}' FORMAT JSON"
        ))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let result: Value = serde_json::from_str(&response)?;
    Ok(serde_json::from_value(result["data"][0].clone())?)
}

async fn query_read_rows(fixture: &SeededDatabase, query_id: &str) -> TestResult<u64> {
    Ok(query_statistics(fixture, query_id).await?.read_rows)
}

async fn sample_timing<Q: litellm_storage_clickhouse::Query<Params = LensSampleParams>>(
    fixture: &SeededDatabase,
    connection: &Connection,
    parameters: &LensSampleParams,
) -> TestResult<f64> {
    let started = std::time::Instant::now();
    let rows =
        litellm_storage_clickhouse::fetch::<Q>(&fixture.database.client, connection, parameters)
            .await?;
    assert_eq!(rows.len(), parameters.limit as usize);
    Ok(started.elapsed().as_secs_f64() * 1000.0)
}

async fn sample_timings<Q: litellm_storage_clickhouse::Query<Params = LensSampleParams>>(
    fixture: &SeededDatabase,
    parameters: &LensSampleParams,
) -> TestResult<(Vec<f64>, Vec<f64>)> {
    use litellm_traces_clickhouse::query::lens::LensSample;
    let connection = Connection::configured(&fixture.database.url, DATABASE, "default", "")?;
    sample_timing::<LensSample>(fixture, &connection, parameters).await?;
    sample_timing::<Q>(fixture, &connection, parameters).await?;
    let mut baseline = Vec::new();
    let mut candidate = Vec::new();
    for sample in 0..11 {
        if sample % 2 == 0 {
            baseline.push(sample_timing::<LensSample>(fixture, &connection, parameters).await?);
            candidate.push(sample_timing::<Q>(fixture, &connection, parameters).await?);
        } else {
            candidate.push(sample_timing::<Q>(fixture, &connection, parameters).await?);
            baseline.push(sample_timing::<LensSample>(fixture, &connection, parameters).await?);
        }
    }
    Ok((baseline, candidate))
}

#[rstest]
#[case::small_payload(3000)]
#[case::large_payload(65_536)]
#[tokio::test]
async fn readiness_counts_without_reading_trace_payloads(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
    #[case] payload_bytes: usize,
) -> TestResult {
    let fixture = migrated_database?;
    seed_payload_days(&fixture, 0, 1, payload_bytes).await?;
    let now_ms = time::OffsetDateTime::now_utc().unix_timestamp() as u64 * 1000;
    let parameters: LensSampleParams = serde_json::from_value(serde_json::to_value(
        sample_parameters(now_ms - 86_400_000, now_ms + 60_000),
    )?)?;
    let parameters = LensSampleParams {
        limit: 1,
        preview: 1,
        ..parameters
    };
    let before_id = format!("lens_readiness_before_{}", std::process::id());
    let before = Connection::configured(
        &format!("{}?query_id={before_id}", fixture.database.url),
        DATABASE,
        "default",
        "",
    )?;
    let baseline = litellm_storage_clickhouse::fetch::<
        litellm_traces_clickhouse::query::lens::LensSample,
    >(&fixture.database.client, &before, &parameters)
    .await?;
    let after_id = format!("lens_readiness_after_{}", std::process::id());
    let after = Connection::configured(
        &format!("{}?query_id={after_id}", fixture.database.url),
        DATABASE,
        "default",
        "",
    )?;
    let counts = litellm_storage_clickhouse::fetch::<LensSampleEligibility>(
        &fixture.database.client,
        &after,
        &parameters,
    )
    .await?;
    assert_eq!(baseline[0].eligible, SPANS_PER_DAY);
    assert_eq!(counts[0].eligible, baseline[0].eligible);
    let before = query_statistics(&fixture, &before_id).await?;
    let after = query_statistics(&fixture, &after_id).await?;
    println!(
        "readiness: before_bytes={} after_bytes={} before_ms={} after_ms={}",
        before.read_bytes, after.read_bytes, before.query_duration_ms, after.query_duration_ms
    );
    assert!(
        after.read_bytes * 8 < before.read_bytes,
        "readiness still reads trace payloads"
    );
    let (baseline, candidate) =
        sample_timings::<LensSampleEligibility>(&fixture, &parameters).await?;
    println!(
        "{}",
        serde_json::json!({"benchmark":"readiness", "traces":SPANS_PER_DAY, "payload_bytes":payload_bytes,
        "baseline_ms":baseline, "candidate_ms":candidate,
        "baseline_read_bytes":before.read_bytes, "candidate_read_bytes":after.read_bytes})
    );
    Ok(())
}

#[rstest]
#[case::matching_metadata("load-team", true, 2000)]
#[case::unmatched_metadata("load-team", false, 0)]
#[case::other_tenant("other-team", true, 0)]
#[tokio::test]
async fn readiness_counts_preserve_metadata_filters_and_scope(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
    #[case] team: &str,
    #[case] matches: bool,
    #[case] expected: u64,
) -> TestResult {
    let fixture = migrated_database?;
    seed_days(&fixture, 0, 1).await?;
    let now_ms = time::OffsetDateTime::now_utc().unix_timestamp() as u64 * 1000;
    let parameters: LensSampleParams = serde_json::from_value(serde_json::to_value(
        sample_parameters(now_ms - 86_400_000, now_ms + 60_000),
    )?)?;
    let parameters = LensSampleParams {
        access: litellm_traces_clickhouse::query::lens::LensAccessParams {
            team: team.into(),
            ..parameters.access
        },
        filter_keys: vec!["payload".into()],
        filter_values: vec![if matches {
            "x".repeat(3000)
        } else {
            "missing".into()
        }],
        limit: 1,
        preview: 1,
        ..parameters
    };
    let connection = Connection::configured(&fixture.database.url, DATABASE, "default", "")?;
    let counts = litellm_storage_clickhouse::fetch::<LensSampleEligibility>(
        &fixture.database.client,
        &connection,
        &parameters,
    )
    .await?;
    assert_eq!(counts.first().map_or(0, |row| row.eligible), expected);
    Ok(())
}

#[rstest]
#[tokio::test]
async fn lens_sample_reads_scale_with_window_not_retention(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
) -> TestResult {
    let fixture = migrated_database?;
    seed_days(&fixture, 0, 8).await?;
    let now_ms = time::OffsetDateTime::now_utc().unix_timestamp() as u64 * 1000;
    let start = now_ms - 86_400_000;
    let end = now_ms + 60_000;
    let before_id = format!("lens_sample_before_{}", std::process::id());
    let (before_rows, response_bytes) = sample(&fixture, start, end, &before_id).await?;
    assert_eq!(before_rows, SPANS_PER_DAY as usize);
    assert!(response_bytes > 4 * 1024 * 1024);
    let before = query_read_rows(&fixture, &before_id).await?;

    seed_days(&fixture, 8, 24).await?;
    let after_id = format!("lens_sample_after_{}", std::process::id());
    let (after_rows, _) = sample(&fixture, start, end, &after_id).await?;
    assert_eq!(after_rows, SPANS_PER_DAY as usize);
    let after = query_read_rows(&fixture, &after_id).await?;
    assert!(
        after * 100 <= before * 105,
        "read_rows grew from {before} to {after}"
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn lens_content_reads_scale_with_trace_not_retention(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
) -> TestResult {
    let fixture = migrated_database?;
    seed_days(&fixture, 0, 8).await?;
    let start_time = trace_start_time(&fixture).await?;
    let before_id = format!("lens_content_before_{}", std::process::id());
    content(&fixture, &start_time, &before_id).await?;
    let before = query_read_rows(&fixture, &before_id).await?;

    seed_days(&fixture, 8, 24).await?;
    let after_id = format!("lens_content_after_{}", std::process::id());
    content(&fixture, &start_time, &after_id).await?;
    let after = query_read_rows(&fixture, &after_id).await?;
    println!("lens_content read_rows: before={before}, after={after}");
    assert!(
        after * 100 <= before * 105,
        "read_rows grew from {before} to {after}"
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn listed_span_reads_skip_unrelated_traces(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
) -> TestResult {
    let fixture = migrated_database?;
    let unrelated = 262_144;
    let start_ms = time::OffsetDateTime::now_utc().unix_timestamp() * 1000;
    let insert = format!(
        "INSERT INTO {DATABASE}.otel_traces \
         (Timestamp, TraceId, SpanId, SpanName, ServiceName, TeamId, ApiKeyHash) \
         SELECT fromUnixTimestamp64Milli({start_ms}), concat('noise-', toString(number)), \
         'span', 'noise', 'service', 'team', 'key' FROM numbers({unrelated})"
    );
    fixture
        .database
        .client
        .post(&fixture.database.url)
        .body(insert)
        .send()
        .await?
        .error_for_status()?;
    let insert = format!(
        "INSERT INTO {DATABASE}.otel_traces \
         (Timestamp, TraceId, SpanId, SpanName, ServiceName, TeamId, ApiKeyHash) \
         VALUES (fromUnixTimestamp64Milli({start_ms}), 'needle', 'span', 'selected', 'service', 'team', 'key'), \
         (fromUnixTimestamp64Milli({start_ms}), 'needle', 'span', 'other tenant', 'service', 'other', 'key')"
    );
    fixture
        .database
        .client
        .post(&fixture.database.url)
        .body(insert)
        .send()
        .await?
        .error_for_status()?;
    let query_id = format!("lens_list_span_index_{}", std::process::id());
    let connection = Connection::configured(
        &format!("{}?query_id={query_id}", fixture.database.url),
        DATABASE,
        "default",
        "",
    )?;
    let store = ClickHouseTraces::new(fixture.database.client.clone(), connection);
    let params = TracePageSpansParams {
        access: ReadAccessParams {
            all_teams: true,
            user_id: String::new(),
            team_ids: Vec::new(),
        },
        trace_refs: vec![format!("{:X}", Sha256::digest(b"team\0key\0needle"))],
        trace_ids: vec!["needle".into()],
        start_ms,
        end_ms: start_ms + 1,
    };
    let started = std::time::Instant::now();
    let rows = store.run_spans(&params, u64::MAX).await?;
    let elapsed = started.elapsed();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "selected");
    let read_rows = query_read_rows(&fixture, &query_id).await?;
    println!(
        "listed span read: rows={read_rows} elapsed_ms={}",
        elapsed.as_secs_f64() * 1000.0
    );
    assert!(
        read_rows < unrelated / 4,
        "read {read_rows} rows for one trace among {unrelated} unrelated rows"
    );
    Ok(())
}

#[rstest]
#[tokio::test]
async fn signal_discovery_avoids_unused_trace_attributes(
    #[future(awt)] migrated_database: TestResult<SeededDatabase>,
) -> TestResult {
    use litellm_traces_clickhouse::query::lens::{LensSample, LensSampleRow};
    let fixture = migrated_database?;
    seed_payload_days(&fixture, 0, 1, 65_536).await?;
    let now_ms = time::OffsetDateTime::now_utc().unix_timestamp() as u64 * 1000;
    let parameters: LensSampleParams = serde_json::from_value(serde_json::to_value(
        sample_parameters(now_ms - 86_400_000, now_ms + 60_000),
    )?)?;
    let parameters = LensSampleParams {
        limit: 100,
        ..parameters
    };
    let before_id = format!("lens_signal_before_{}", std::process::id());
    let before = Connection::configured(
        &format!("{}?query_id={before_id}", fixture.database.url),
        DATABASE,
        "default",
        "",
    )?;
    let baseline = litellm_storage_clickhouse::fetch::<LensSample>(
        &fixture.database.client,
        &before,
        &parameters,
    )
    .await?;
    let after_id = format!("lens_signal_after_{}", std::process::id());
    let after = Connection::configured(
        &format!("{}?query_id={after_id}", fixture.database.url),
        DATABASE,
        "default",
        "",
    )?;
    let candidate = litellm_storage_clickhouse::fetch::<LensSignalSample>(
        &fixture.database.client,
        &after,
        &parameters,
    )
    .await?;
    assert_eq!(candidate.len(), 100);
    assert_eq!(candidate[0].eligible, SPANS_PER_DAY);
    let expected: Vec<_> = baseline
        .into_iter()
        .map(|row| LensSampleRow {
            attributes: Vec::new(),
            ..row
        })
        .collect();
    assert_eq!(
        serde_json::to_value(&candidate)?,
        serde_json::to_value(&expected)?
    );
    let before = query_statistics(&fixture, &before_id).await?;
    let after = query_statistics(&fixture, &after_id).await?;
    assert!(
        after.read_bytes * 8 < before.read_bytes,
        "signal discovery still reads trace payloads"
    );
    let (baseline, candidate) = sample_timings::<LensSignalSample>(&fixture, &parameters).await?;
    println!(
        "{}",
        serde_json::json!({"benchmark":"signal_discovery", "traces":SPANS_PER_DAY, "payload_bytes":65536,
        "baseline_ms":baseline, "candidate_ms":candidate,
        "baseline_read_bytes":before.read_bytes, "candidate_read_bytes":after.read_bytes})
    );
    Ok(())
}
