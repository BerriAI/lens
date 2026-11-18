use std::collections::{BTreeMap, BTreeSet};

use lens_contract::ledger::LedgerRejection;
use litellm_http::Client;
use litellm_traces::Shared;
use serde_json::Value;

use crate::{Connection, Error, InsertRow, cost_rows::upstream, insert::insert_into};

const SPEND_TABLE: &str = "spend_logs";
const COSTS_TABLE: &str = "lens_ledger_costs";
const ALIASES_TABLE: &str = "lens_ledger_aliases";
const REFS_TABLE: &str = "lens_payload_refs";

const TEXT_COLUMNS: [&str; 25] = [
    "request_id",
    "response_id",
    "provider_request_id",
    "litellm_call_id",
    "call_type",
    "api_key",
    "key_alias",
    "team_id",
    "team_alias",
    "organization_id",
    "user",
    "end_user",
    "model",
    "model_group",
    "model_id",
    "custom_llm_provider",
    "api_base",
    "status",
    "error_str",
    "session_id",
    "trace_id",
    "span_id",
    "metadata",
    "messages",
    "response",
];

const COUNT_COLUMNS: [&str; 5] = [
    "prompt_tokens",
    "completion_tokens",
    "total_tokens",
    "cache_read_tokens",
    "cache_write_tokens",
];

const CONTENT_COLUMNS: [&str; 2] = ["messages", "response"];

const COST_COLUMNS: [&str; 18] = [
    "request_id",
    "litellm_call_id",
    "response_id",
    "provider_request_id",
    "trace_id",
    "span_id",
    "session_id",
    "api_key",
    "user",
    "organization_id",
    "model",
    "custom_llm_provider",
    "call_type",
    "status",
    "team_id",
    "spend",
    "prompt_tokens",
    "completion_tokens",
];

#[derive(Clone, Debug)]
pub struct LedgerRow {
    spend: InsertRow,
    cost: InsertRow,
    aliases: Vec<InsertRow>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayloadRefRow {
    pub team_id: String,
    pub api_key: String,
    pub event_id: String,
    pub field: &'static str,
    pub event_time_ms: i64,
    pub object_key: String,
    pub bytes: u64,
    pub sha256: String,
}

fn shared(value: impl Into<Value>) -> Shared<Value> {
    Shared::new(value.into())
}

fn text(record: &BTreeMap<String, Value>, name: &str) -> Result<String, LedgerRejection> {
    match record.get(name) {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(value)) => Ok(value.clone()),
        Some(_) => Err(LedgerRejection::InvalidRecord),
    }
}

fn count(record: &BTreeMap<String, Value>, name: &str) -> Result<u64, LedgerRejection> {
    match record.get(name) {
        None | Some(Value::Null) => Ok(0),
        Some(value) => value
            .as_u64()
            .filter(|count| *count <= u64::from(u32::MAX))
            .ok_or(LedgerRejection::InvalidRecord),
    }
}

fn millis(record: &BTreeMap<String, Value>, name: &str) -> Result<Option<i64>, LedgerRejection> {
    match record.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .filter(|ms| *ms >= 0)
            .map(Some)
            .ok_or(LedgerRejection::InvalidRecord),
    }
}

fn spend(record: &BTreeMap<String, Value>) -> Result<Value, LedgerRejection> {
    match record.get("spend") {
        None | Some(Value::Null) => Ok(Value::Null),
        Some(value) if value.as_f64().is_some_and(f64::is_finite) => Ok(value.clone()),
        Some(_) => Err(LedgerRejection::InvalidRecord),
    }
}

fn tags(record: &BTreeMap<String, Value>) -> Result<Value, LedgerRejection> {
    match record.get("request_tags") {
        None | Some(Value::Null) => Ok(Value::Array(Vec::new())),
        Some(Value::Array(items)) if items.iter().all(Value::is_string) => {
            Ok(Value::Array(items.clone()))
        }
        Some(_) => Err(LedgerRejection::InvalidRecord),
    }
}

fn cache_hit(record: &BTreeMap<String, Value>) -> Result<bool, LedgerRejection> {
    match record.get("cache_hit") {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(LedgerRejection::InvalidRecord),
    }
}

fn bounded(name: &str, value: String, inline_bytes: usize) -> String {
    if CONTENT_COLUMNS.contains(&name) && value.len() > inline_bytes {
        lens_payloads::preview(&value, inline_bytes)
    } else {
        value
    }
}

/// Projects one accepted gateway delivery into its spend log row, its compact cost row keyed by
/// `event_id`, and narrow alias rows that point at that event.
pub fn ledger_row(
    event_id: &str,
    record: &BTreeMap<String, Value>,
    inline_bytes: usize,
) -> Result<LedgerRow, LedgerRejection> {
    if !lens_payloads::valid_event_id(event_id) {
        return Err(LedgerRejection::InvalidEventId);
    }
    let start_ms = millis(record, "start_time")
        .ok()
        .flatten()
        .filter(|ms| *ms > 0)
        .ok_or(LedgerRejection::MissingEventTime)?;
    let end_ms = millis(record, "end_time")?.unwrap_or(start_ms);
    let texts = TEXT_COLUMNS
        .into_iter()
        .map(|name| text(record, name).map(|value| (name, bounded(name, value, inline_bytes))))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    if texts["request_id"].is_empty() {
        return Err(LedgerRejection::InvalidRecord);
    }
    let counts = COUNT_COLUMNS
        .into_iter()
        .map(|name| count(record, name).map(|value| (name, value)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let spend = spend(record)?;
    let spend_row: InsertRow = texts
        .iter()
        .map(|(name, value)| ((*name).to_owned(), shared(value.clone())))
        .chain(
            counts
                .iter()
                .map(|(name, value)| ((*name).to_owned(), shared(*value))),
        )
        .chain([
            ("ledger_event_id".to_owned(), shared(event_id)),
            ("spend".to_owned(), Shared::new(spend.clone())),
            ("start_time".to_owned(), shared(start_ms)),
            ("end_time".to_owned(), shared(end_ms)),
            (
                "completion_start_time".to_owned(),
                millis(record, "completion_start_time")?
                    .map_or_else(|| Shared::new(Value::Null), shared),
            ),
            ("cache_hit".to_owned(), shared(cache_hit(record)?)),
            ("request_tags".to_owned(), Shared::new(tags(record)?)),
        ])
        .collect();
    let upstream_response_id = upstream(&texts["response_id"]);
    let cost: InsertRow = COST_COLUMNS
        .into_iter()
        .filter_map(|name| {
            spend_row
                .get(name)
                .map(|value| (name.to_owned(), value.clone()))
        })
        .chain([
            ("event_id".to_owned(), shared(event_id)),
            ("event_time".to_owned(), shared(start_ms)),
            (
                "upstream_response_id".to_owned(),
                shared(upstream_response_id.clone()),
            ),
        ])
        .collect();
    let request = texts["request_id"].as_str();
    let gateway = texts["litellm_call_id"].as_str();
    let keys: BTreeSet<(&str, &str)> = [
        ("canonical", request),
        (
            "litellm_request",
            if gateway.is_empty() { request } else { gateway },
        ),
        ("provider_response", texts["response_id"].as_str()),
        ("provider_response", upstream_response_id.as_str()),
        ("provider_request", texts["provider_request_id"].as_str()),
        ("transport", texts["trace_id"].as_str()),
    ]
    .into_iter()
    .filter(|(_, value)| !value.is_empty())
    .collect();
    let aliases = keys
        .into_iter()
        .map(|(kind, value)| {
            InsertRow::from([
                ("key_kind".to_owned(), shared(kind)),
                ("key_value".to_owned(), shared(value)),
                ("team_id".to_owned(), shared(texts["team_id"].clone())),
                ("event_time".to_owned(), shared(start_ms)),
                ("event_id".to_owned(), shared(event_id)),
                ("api_key".to_owned(), shared(texts["api_key"].clone())),
                ("user".to_owned(), shared(texts["user"].clone())),
            ])
        })
        .collect();
    Ok(LedgerRow {
        spend: spend_row,
        cost,
        aliases,
    })
}

/// Writes spend rows and aliases before cost rows, so a cost becomes readable only once the rows
/// that resolve it exist. Every write is replay-safe: a retried batch collapses on the same keys.
pub async fn insert_ledger(
    client: &Client,
    connection: &Connection,
    database: &str,
    rows: &[LedgerRow],
) -> Result<(), Error> {
    let spend: Vec<InsertRow> = rows.iter().map(|row| row.spend.clone()).collect();
    let aliases: Vec<InsertRow> = rows.iter().flat_map(|row| row.aliases.clone()).collect();
    let costs: Vec<InsertRow> = rows.iter().map(|row| row.cost.clone()).collect();
    insert_into(client, connection, database, SPEND_TABLE, &spend).await?;
    insert_into(client, connection, database, ALIASES_TABLE, &aliases).await?;
    insert_into(client, connection, database, COSTS_TABLE, &costs).await
}

pub async fn insert_payload_ref(
    client: &Client,
    connection: &Connection,
    database: &str,
    reference: &PayloadRefRow,
) -> Result<(), Error> {
    let row = InsertRow::from([
        ("team_id".to_owned(), shared(reference.team_id.clone())),
        ("api_key".to_owned(), shared(reference.api_key.clone())),
        ("event_id".to_owned(), shared(reference.event_id.clone())),
        ("field".to_owned(), shared(reference.field)),
        ("event_time".to_owned(), shared(reference.event_time_ms)),
        (
            "object_key".to_owned(),
            shared(reference.object_key.clone()),
        ),
        ("bytes".to_owned(), shared(reference.bytes)),
        ("sha256".to_owned(), shared(reference.sha256.clone())),
    ]);
    insert_into(client, connection, database, REFS_TABLE, &[row]).await
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde_json::json;

    use super::*;

    fn record(fields: Value) -> BTreeMap<String, Value> {
        serde_json::from_value(fields).unwrap()
    }

    fn valid() -> BTreeMap<String, Value> {
        record(json!({
            "request_id": "req", "litellm_call_id": "call", "response_id": "chatcmpl-1",
            "team_id": "team", "api_key": "hash", "user": "u", "spend": 0.5,
            "start_time": 1_767_225_600_000_i64, "end_time": 1_767_225_601_000_i64,
            "messages": "[]", "response": "{}", "unknown_future_field": 1,
        }))
    }

    #[rstest]
    fn cost_row_is_keyed_by_event_and_event_time() {
        let row = ledger_row("evt-1", &valid(), 1024).unwrap();

        assert_eq!(*row.cost["event_id"], json!("evt-1"));
        assert_eq!(*row.cost["event_time"], json!(1_767_225_600_000_i64));
        assert_eq!(*row.cost["spend"], json!(0.5));
        assert_eq!(*row.cost["request_id"], json!("req"));
        assert!(!row.cost.contains_key("messages"));
        assert_eq!(*row.spend["ledger_event_id"], json!("evt-1"));
        assert!(!row.spend.contains_key("unknown_future_field"));
    }

    #[rstest]
    fn aliases_are_narrow_and_point_at_the_event() {
        let row = ledger_row("evt-1", &valid(), 1024).unwrap();
        let kinds: Vec<(String, String)> = row
            .aliases
            .iter()
            .map(|alias| {
                assert_eq!(*alias["event_id"], json!("evt-1"));
                assert!(!alias.contains_key("spend"));
                (
                    alias["key_kind"].as_str().unwrap().to_owned(),
                    alias["key_value"].as_str().unwrap().to_owned(),
                )
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("canonical".to_owned(), "req".to_owned()),
                ("litellm_request".to_owned(), "call".to_owned()),
                ("provider_response".to_owned(), "chatcmpl-1".to_owned()),
            ]
        );
    }

    #[rstest]
    #[case::missing_time(json!({"start_time": null}), LedgerRejection::MissingEventTime)]
    #[case::zero_time(json!({"start_time": 0}), LedgerRejection::MissingEventTime)]
    #[case::text_type(json!({"team_id": 3}), LedgerRejection::InvalidRecord)]
    #[case::token_overflow(json!({"prompt_tokens": 4_294_967_296_u64}), LedgerRejection::InvalidRecord)]
    #[case::negative_tokens(json!({"prompt_tokens": -1}), LedgerRejection::InvalidRecord)]
    #[case::spend_type(json!({"spend": "0.5"}), LedgerRejection::InvalidRecord)]
    #[case::tags(json!({"request_tags": [1]}), LedgerRejection::InvalidRecord)]
    #[case::empty_request(json!({"request_id": ""}), LedgerRejection::InvalidRecord)]
    fn invalid_records_are_rejected_without_poisoning_the_batch(
        #[case] change: Value,
        #[case] expected: LedgerRejection,
    ) {
        let merged = valid()
            .into_iter()
            .chain(record(change))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(ledger_row("evt", &merged, 1024).unwrap_err(), expected);
    }

    #[rstest]
    #[case::slash("a/b")]
    #[case::empty("")]
    fn event_ids_must_be_safe_identifiers(#[case] event_id: &str) {
        assert_eq!(
            ledger_row(event_id, &valid(), 1024).unwrap_err(),
            LedgerRejection::InvalidEventId
        );
    }

    #[rstest]
    #[case::at_limit(300, false)]
    #[case::over_limit(301, true)]
    fn content_above_the_inline_limit_is_never_stored_raw(
        #[case] length: usize,
        #[case] truncated: bool,
    ) {
        let body = "雪".repeat(100);
        let content = format!("{body}{}", "x".repeat(length - body.len()));
        let merged = valid()
            .into_iter()
            .chain([("messages".to_owned(), json!(content))])
            .collect::<BTreeMap<_, _>>();
        let row = ledger_row("evt", &merged, 300).unwrap();
        let stored = row.spend["messages"].as_str().unwrap();

        assert!(stored.len() <= 300);
        assert_eq!(stored == content, !truncated);
        assert_eq!(stored.contains("bytes omitted]"), truncated);
    }
}
