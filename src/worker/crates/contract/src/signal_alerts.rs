use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SignalAlertRequest {
    pub revision: u64,
    pub enabled: bool,
    pub signal_ids: Vec<String>,
    #[serde(default)]
    pub webhook_url: Option<String>,
}

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SignalAlertTest {
    #[serde(default)]
    pub webhook_url: Option<String>,
}

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
pub struct SignalAlertView {
    pub revision: u64,
    pub enabled: bool,
    pub signal_ids: Vec<String>,
    pub connected: bool,
    pub last_delivery: Option<SignalAlertDelivery>,
}

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
pub struct SignalAlertDelivery {
    pub at: DateTime<Utc>,
    pub status: String,
}
