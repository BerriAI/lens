use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct IngestionKeyRequest {
    #[schemars(length(min = 1, max = 128))]
    pub name: String,
    #[schemars(length(max = 256))]
    pub team_id: String,
    pub expires_at: Option<DateTime<Utc>>,
}

impl Default for IngestionKeyRequest {
    fn default() -> Self {
        Self {
            name: "Agent tracing".into(),
            team_id: String::new(),
            expires_at: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IngestionTenant {
    #[serde(default)]
    pub team_id: String,
    pub user_id: String,
    #[serde(default)]
    pub org_id: String,
    pub api_key_hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IngestionKey {
    pub id: String,
    pub name: String,
    pub tenant: IngestionTenant,
    pub created_at: DateTime<Utc>,
    #[serde(deserialize_with = "Option::deserialize")]
    pub expires_at: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IngestionCredential {
    pub token_hash: String,
    pub tenant: IngestionTenant,
    #[serde(deserialize_with = "Option::deserialize")]
    pub expires_at: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IngestionSnapshot {
    pub issued_at: i64,
    pub keys: Vec<IngestionCredential>,
}

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IngestionKeyCreated {
    pub key: String,
    pub record: IngestionKey,
    #[serde(default)]
    pub active: bool,
}
