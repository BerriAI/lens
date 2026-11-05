use std::collections::BTreeMap;

use litellm_http::Client;
use litellm_storage_clickhouse::{Query, ReadLimits, fetch};
use litellm_traces::{LogListRequest, LogValue, OtelLogPage, OtelLogRecord, OtelLogSummary};
use serde::{Deserialize, Serialize};

use crate::{Connection, Error, query::number};

const LOG_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const LOG_PAGE_SIZE_MAX: u16 = 100;

#[derive(Serialize)]
struct ListLogsParams {
    start_ms: i64,
    end_ms: i64,
    service: String,
    event: String,
    severity: String,
    trace_id: String,
    conversation_id: String,
    has_cursor: u8,
    cursor_timestamp_ns: i64,
    cursor_record_id: String,
    limit: u32,
}

#[derive(Deserialize)]
struct LogSummaryRow {
    record_id: String,
    #[serde(deserialize_with = "number::deserialize")]
    timestamp_ns: u64,
    #[serde(deserialize_with = "number::deserialize")]
    observed_timestamp_ns: u64,
    severity_number: i32,
    severity_text: String,
    event_name: String,
    semantic_event_name: Option<String>,
    scope_name: String,
    service_name: String,
    trace_id: String,
    span_id: String,
    conversation_id: Option<String>,
}

struct ListLogs;

impl Query for ListLogs {
    type Params = ListLogsParams;
    type Row = LogSummaryRow;

    const READ_LIMITS: ReadLimits = ReadLimits {
        result_rows: 101,
        response_bytes: LOG_RESPONSE_BYTES,
        execution_seconds: 10,
    };
    const SQL: &'static str = include_str!("../query/list_logs.sql");
}

#[derive(Serialize)]
struct GetLogParams {
    record_id: String,
}

#[derive(Deserialize)]
struct LogRow {
    record_id: String,
    #[serde(deserialize_with = "number::deserialize")]
    timestamp_ns: u64,
    #[serde(deserialize_with = "number::deserialize")]
    raw_timestamp_ns: u64,
    #[serde(deserialize_with = "number::deserialize")]
    observed_timestamp_ns: u64,
    severity_number: i32,
    severity_text: String,
    event_name: String,
    semantic_event_name: Option<String>,
    body: String,
    body_present: bool,
    attributes: String,
    resource_attributes: String,
    resource_schema_url: String,
    scope_name: String,
    scope_version: String,
    scope_attributes: String,
    scope_schema_url: String,
    trace_id: String,
    span_id: String,
    #[serde(deserialize_with = "number::deserialize")]
    trace_flags: u32,
    #[serde(deserialize_with = "number::deserialize")]
    dropped_attributes_count: u32,
    service_name: String,
    conversation_id: Option<String>,
    codex_projection: String,
}

struct GetLog;

impl Query for GetLog {
    type Params = GetLogParams;
    type Row = LogRow;

    const READ_LIMITS: ReadLimits = ReadLimits {
        result_rows: 1,
        response_bytes: LOG_RESPONSE_BYTES,
        execution_seconds: 10,
    };
    const SQL: &'static str = include_str!("../query/get_log.sql");
}

pub async fn list_logs(
    client: &Client,
    connection: &Connection,
    request: LogListRequest,
) -> Result<OtelLogPage, Error> {
    let start_ms = request.start_ms.ok_or(Error::InvalidParameters)?;
    let end_ms = request.end_ms.ok_or(Error::InvalidParameters)?;
    let page_size = request.page_size.ok_or(Error::InvalidParameters)?;
    if start_ms >= end_ms || !(1..=LOG_PAGE_SIZE_MAX).contains(&page_size) {
        return Err(Error::InvalidParameters);
    }
    let cursor = request.cursor.as_deref().map(parse_cursor).transpose()?;
    let rows = fetch::<ListLogs>(
        client,
        connection,
        &ListLogsParams {
            start_ms,
            end_ms,
            service: request.service.unwrap_or_default(),
            event: request.event.unwrap_or_default(),
            severity: request.severity.unwrap_or_default(),
            trace_id: request.trace_id.unwrap_or_default(),
            conversation_id: request.conversation_id.unwrap_or_default(),
            has_cursor: u8::from(cursor.is_some()),
            cursor_timestamp_ns: cursor.as_ref().map_or(0, |(timestamp, _)| *timestamp),
            cursor_record_id: cursor.map_or_else(String::new, |(_, record_id)| record_id),
            limit: u32::from(page_size) + 1,
        },
    )
    .await?;
    let has_more = rows.len() > usize::from(page_size);
    let data: Vec<_> = rows
        .into_iter()
        .take(usize::from(page_size))
        .map(|row| OtelLogSummary {
            record_id: row.record_id,
            timestamp_ns: row.timestamp_ns.to_string(),
            observed_timestamp_ns: row.observed_timestamp_ns.to_string(),
            severity_number: row.severity_number,
            severity_text: row.severity_text,
            event_name: row.event_name,
            semantic_event_name: row.semantic_event_name,
            scope_name: row.scope_name,
            service_name: row.service_name,
            trace_id: row.trace_id,
            span_id: row.span_id,
            conversation_id: row.conversation_id,
        })
        .collect();
    let next_cursor = has_more
        .then(|| data.last())
        .flatten()
        .map(|row| format!("{}:{}", row.timestamp_ns, row.record_id));
    Ok(OtelLogPage { data, next_cursor })
}

pub async fn get_log(
    client: &Client,
    connection: &Connection,
    record_id: String,
) -> Result<Option<OtelLogRecord>, Error> {
    if !valid_record_id(&record_id) {
        return Err(Error::InvalidParameters);
    }
    fetch::<GetLog>(client, connection, &GetLogParams { record_id })
        .await?
        .into_iter()
        .next()
        .map(decode_log)
        .transpose()
}

fn parse_cursor(cursor: &str) -> Result<(i64, String), Error> {
    let (timestamp, record_id) = cursor.split_once(':').ok_or(Error::InvalidParameters)?;
    let timestamp = timestamp
        .parse::<i64>()
        .ok()
        .filter(|timestamp| *timestamp >= 0)
        .ok_or(Error::InvalidParameters)?;
    if !valid_record_id(record_id) {
        return Err(Error::InvalidParameters);
    }
    Ok((timestamp, record_id.to_owned()))
}

fn valid_record_id(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn decode_json<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, Error> {
    serde_json::from_str(value).map_err(|_| Error::InvalidResponse)
}

fn decode_log(row: LogRow) -> Result<OtelLogRecord, Error> {
    Ok(OtelLogRecord {
        record_id: row.record_id,
        timestamp_ns: row.timestamp_ns.to_string(),
        raw_timestamp_ns: row.raw_timestamp_ns.to_string(),
        observed_timestamp_ns: row.observed_timestamp_ns.to_string(),
        severity_number: row.severity_number,
        severity_text: row.severity_text,
        event_name: row.event_name,
        semantic_event_name: row.semantic_event_name,
        body: row
            .body_present
            .then(|| decode_json::<LogValue>(&row.body))
            .transpose()?,
        attributes: decode_json::<BTreeMap<String, LogValue>>(&row.attributes)?,
        resource_attributes: decode_json::<BTreeMap<String, LogValue>>(&row.resource_attributes)?,
        resource_schema_url: row.resource_schema_url,
        scope_name: row.scope_name,
        scope_version: row.scope_version,
        scope_attributes: decode_json::<BTreeMap<String, LogValue>>(&row.scope_attributes)?,
        scope_schema_url: row.scope_schema_url,
        trace_id: row.trace_id,
        span_id: row.span_id,
        trace_flags: row.trace_flags,
        dropped_attributes_count: row.dropped_attributes_count,
        service_name: row.service_name,
        conversation_id: row.conversation_id,
        codex: decode_json(&row.codex_projection)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::valid("123:".to_owned() + &"a".repeat(64), true)]
    #[case::negative("-1:".to_owned() + &"a".repeat(64), false)]
    #[case::wrong_id_length("123:abc".to_owned(), false)]
    #[case::invalid_id("123:".to_owned() + &"g".repeat(64), false)]
    fn cursors_require_a_timestamp_and_record_identity(
        #[case] cursor: String,
        #[case] valid: bool,
    ) {
        assert_eq!(parse_cursor(&cursor).is_ok(), valid);
    }
}
