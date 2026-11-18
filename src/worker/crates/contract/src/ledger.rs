use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const LEDGER_VERSION: u32 = 1;
pub const SUPPORTED_LEDGER_VERSIONS: [u32; 1] = [LEDGER_VERSION];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub struct LedgerEnvelope {
    pub version: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LedgerBatch {
    pub version: u32,
    #[schemars(length(max = 1000))]
    pub events: Vec<LedgerEvent>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LedgerEvent {
    #[schemars(length(min = 1, max = 128))]
    pub event_id: String,
    pub record: BTreeMap<String, Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LedgerStatus {
    Stored,
    Rejected,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LedgerRejection {
    InvalidEventId,
    MissingEventTime,
    InvalidRecord,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LedgerResult {
    pub event_id: String,
    pub status: LedgerStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<LedgerRejection>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LedgerResults {
    pub version: u32,
    pub results: Vec<LedgerResult>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PayloadUpload {
    pub version: u32,
    #[schemars(length(min = 1, max = 128))]
    pub event_id: String,
    pub field: String,
    pub team_id: String,
    pub api_key: String,
    pub event_time_ms: i64,
    pub bytes: u64,
    #[schemars(length(min = 64, max = 64))]
    pub sha256: String,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LedgerCapabilities {
    pub versions: Vec<u32>,
    pub payload_store: bool,
    pub inline_bytes: u64,
    pub preview_bytes: u64,
}
