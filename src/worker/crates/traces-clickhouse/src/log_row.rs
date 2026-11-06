use std::collections::BTreeMap;

use litellm_traces::{DecodedLogRecord, LogValue, Shared, SharedIdentity, Tenant};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{Error, InsertRow};

struct SharedValues<T>(std::collections::HashMap<SharedIdentity, (Shared<T>, Shared<Value>)>);

impl<T: Clone> SharedValues<T> {
    fn new() -> Self {
        Self(std::collections::HashMap::new())
    }

    fn get(
        &mut self,
        source: &Shared<T>,
        convert: impl FnOnce(&T) -> Result<Value, Error>,
    ) -> Result<Shared<Value>, Error> {
        if let Some((_, value)) = self.0.get(&source.identity()) {
            return Ok(value.clone());
        }
        let value = Shared::new(convert(source)?);
        self.0
            .insert(source.identity(), (source.clone(), value.clone()));
        Ok(value)
    }
}

fn json_string<T: Serialize>(value: &T) -> Result<Value, Error> {
    serde_json::to_string(value)
        .map(Value::String)
        .map_err(|_| Error::InvalidRow)
}

fn string_attribute(attributes: &BTreeMap<String, LogValue>, key: &str) -> String {
    match attributes.get(key) {
        Some(LogValue::String(value)) => value.clone(),
        _ => String::new(),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn record_id(record: &DecodedLogRecord, tenant: &Tenant) -> Result<String, Error> {
    let canonical = serde_json::to_vec(record).map_err(|_| Error::InvalidRow)?;
    let mut hash = Sha256::new();
    hash.update(b"litellm.otel-log.v1\0");
    for value in [
        tenant.team_id.as_bytes(),
        tenant.api_key_hash.as_bytes(),
        tenant.org_id.as_bytes(),
        tenant.user_id.as_bytes(),
        canonical.as_slice(),
    ] {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub fn log_rows(records: Vec<DecodedLogRecord>, tenant: &Tenant) -> Result<Vec<InsertRow>, Error> {
    let mut resources = SharedValues::new();
    let mut scopes = SharedValues::new();
    records
        .into_iter()
        .map(|record| {
            let resource_attributes = resources.get(&record.resource_attributes, json_string)?;
            let scope_attributes = scopes.get(&record.scope_attributes, json_string)?;
            let body_present = record.body.is_some();
            let body = json_string(&record.body)?;
            let attributes = json_string(&record.attributes)?;
            let semantic_event_name = record.semantic_event_name.clone().unwrap_or_default();
            let framework = if record.codex.is_some() {
                "openai-codex"
            } else {
                ""
            };
            let codex_projection = json_string(&record.codex)?;
            let values = [
                (
                    "RecordId",
                    Shared::new(Value::String(record_id(&record, tenant)?)),
                ),
                (
                    "Timestamp",
                    Shared::new(Value::from(record.effective_timestamp_ns)),
                ),
                (
                    "RawTimestamp",
                    Shared::new(Value::from(record.timestamp_ns)),
                ),
                (
                    "ObservedTimestamp",
                    Shared::new(Value::from(record.observed_timestamp_ns)),
                ),
                (
                    "SeverityNumber",
                    Shared::new(Value::from(record.severity_number)),
                ),
                (
                    "SeverityText",
                    Shared::new(Value::String(record.severity_text)),
                ),
                ("EventName", Shared::new(Value::String(record.event_name))),
                (
                    "SemanticEventName",
                    Shared::new(Value::String(semantic_event_name)),
                ),
                (
                    "Framework",
                    Shared::new(Value::String(framework.to_owned())),
                ),
                ("CodexProjection", Shared::new(codex_projection)),
                ("Body", Shared::new(body)),
                ("BodyPresent", Shared::new(Value::Bool(body_present))),
                ("Attributes", Shared::new(attributes)),
                ("ResourceAttributes", resource_attributes),
                (
                    "ResourceSchemaUrl",
                    Shared::new(Value::String(record.resource_schema_url)),
                ),
                (
                    "ScopeName",
                    Shared::new(Value::String(record.scope_name.to_string())),
                ),
                (
                    "ScopeVersion",
                    Shared::new(Value::String(record.scope_version.to_string())),
                ),
                ("ScopeAttributes", scope_attributes),
                (
                    "ScopeSchemaUrl",
                    Shared::new(Value::String(record.scope_schema_url)),
                ),
                ("TraceId", Shared::new(Value::String(hex(&record.trace_id)))),
                ("SpanId", Shared::new(Value::String(hex(&record.span_id)))),
                ("TraceFlags", Shared::new(Value::from(record.trace_flags))),
                (
                    "DroppedAttributesCount",
                    Shared::new(Value::from(record.dropped_attributes_count)),
                ),
                (
                    "ServiceName",
                    Shared::new(Value::String(string_attribute(
                        &record.resource_attributes,
                        "service.name",
                    ))),
                ),
                (
                    "ConversationId",
                    Shared::new(Value::String(string_attribute(
                        &record.attributes,
                        "conversation.id",
                    ))),
                ),
                ("TeamId", Shared::new(Value::from(tenant.team_id.as_str()))),
                (
                    "ApiKeyHash",
                    Shared::new(Value::from(tenant.api_key_hash.as_str())),
                ),
                ("OrgId", Shared::new(Value::from(tenant.org_id.as_str()))),
                ("UserId", Shared::new(Value::from(tenant.user_id.as_str()))),
            ];
            Ok(values
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value))
                .collect())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use litellm_traces::Shared;
    use rstest::rstest;

    fn record() -> DecodedLogRecord {
        DecodedLogRecord {
            timestamp_ns: 1,
            effective_timestamp_ns: 1,
            observed_timestamp_ns: 2,
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
            scope_version: Shared::new("1.0".to_owned()),
            scope_attributes: Shared::new(BTreeMap::new()),
            resource_schema_url: "resource-schema".to_owned(),
            scope_schema_url: "scope-schema".to_owned(),
            trace_id: vec![1; 16],
            span_id: vec![2; 8],
            trace_flags: 1,
            dropped_attributes_count: 0,
            codex: None,
        }
    }

    fn tenant() -> Tenant {
        Tenant {
            team_id: "team".to_owned(),
            api_key_hash: "key-hash".to_owned(),
            org_id: "org".to_owned(),
            user_id: "user".to_owned(),
        }
    }

    fn field<'a>(row: &'a InsertRow, name: &str) -> &'a Value {
        row.get(name).expect("log row field").as_ref()
    }

    #[rstest]
    fn raw_log_rows_preserve_types_and_use_stable_tenant_scoped_ids() {
        let tenant = tenant();
        let first = log_rows(vec![record()], &tenant).unwrap();
        let repeated = log_rows(vec![record()], &tenant).unwrap();
        assert_eq!(
            field(&first[0], "RecordId"),
            field(&repeated[0], "RecordId")
        );
        assert_eq!(
            serde_json::from_str::<Value>(field(&first[0], "Attributes").as_str().unwrap())
                .unwrap()["attempt"],
            serde_json::json!({"type": "integer", "value": 3})
        );
        assert_eq!(
            serde_json::from_str::<Value>(field(&first[0], "Body").as_str().unwrap()).unwrap(),
            serde_json::json!({"type": "bytes", "value": [255]})
        );
        assert_eq!(
            serde_json::from_str::<Value>(field(&first[0], "ResourceAttributes").as_str().unwrap())
                .unwrap()["service.name"],
            serde_json::json!({"type": "string", "value": "codex_cli_rs"})
        );
        assert_eq!(field(&first[0], "ServiceName"), "codex_cli_rs");
        assert_eq!(field(&first[0], "ConversationId"), "conversation-one");
        let event_time_row = log_rows(
            vec![DecodedLogRecord {
                timestamp_ns: 0,
                effective_timestamp_ns: 42,
                ..record()
            }],
            &tenant,
        )
        .unwrap();
        assert_eq!(field(&event_time_row[0], "Timestamp"), &Value::from(42));
        assert_eq!(field(&event_time_row[0], "RawTimestamp"), &Value::from(0));
        assert_ne!(
            field(&first[0], "RecordId"),
            field(
                &log_rows(vec![record()], &Tenant::default()).unwrap()[0],
                "RecordId"
            )
        );
    }

    #[rstest]
    fn raw_log_identity_changes_with_source_evidence() {
        let original = field(&log_rows(vec![record()], &tenant()).unwrap()[0], "RecordId").clone();
        let changed_timestamp = DecodedLogRecord {
            timestamp_ns: 2,
            ..record()
        };
        assert_ne!(
            original,
            field(
                &log_rows(vec![changed_timestamp], &tenant()).unwrap()[0],
                "RecordId"
            )
            .clone()
        );
        let changed_observed_timestamp = DecodedLogRecord {
            observed_timestamp_ns: 3,
            ..record()
        };
        assert_ne!(
            original,
            field(
                &log_rows(vec![changed_observed_timestamp], &tenant()).unwrap()[0],
                "RecordId"
            )
            .clone()
        );
        let changed_event = DecodedLogRecord {
            semantic_event_name: Some("codex.tool_result".to_owned()),
            ..record()
        };
        assert_ne!(
            original,
            field(
                &log_rows(vec![changed_event], &tenant()).unwrap()[0],
                "RecordId"
            )
            .clone()
        );
        let changed_conversation = DecodedLogRecord {
            attributes: BTreeMap::from([
                (
                    "conversation.id".to_owned(),
                    LogValue::String("conversation-two".to_owned()),
                ),
                ("attempt".to_owned(), LogValue::Integer(3)),
            ]),
            ..record()
        };
        assert_ne!(
            original,
            field(
                &log_rows(vec![changed_conversation], &tenant()).unwrap()[0],
                "RecordId"
            )
            .clone()
        );
        let changed_trace = DecodedLogRecord {
            trace_id: vec![9; 16],
            ..record()
        };
        assert_ne!(
            original,
            field(
                &log_rows(vec![changed_trace], &tenant()).unwrap()[0],
                "RecordId"
            )
            .clone()
        );
    }
}
