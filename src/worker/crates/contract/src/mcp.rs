use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpAuthentication {
    Gateway,
    Token,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpConnection {
    pub id: String,
    pub name: String,
    pub client: String,
    pub owner_id: String,
    pub owner_name: String,
    pub authentication: McpAuthentication,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpCreateRequest {
    pub request_id: String,
    pub name: String,
    pub client: String,
    #[serde(default)]
    pub owner_label: Option<String>,
    #[serde(default = "default_expiry")]
    pub expires_in_days: u16,
}

fn default_expiry() -> u16 {
    90
}

#[derive(Serialize, JsonSchema)]
pub struct McpConnectionCreated {
    pub connection: McpConnection,
    pub token: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpOutcome {
    Success,
    ToolError,
    InvalidRequest,
    Unauthorized,
    Forbidden,
    Expired,
    Revoked,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct McpCall {
    pub id: String,
    pub at_ms: i64,
    pub connection_id: String,
    pub connection_name: String,
    pub owner_id: String,
    pub owner_name: String,
    pub client: String,
    pub method: String,
    pub tool: Option<String>,
    pub outcome: McpOutcome,
    pub latency_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
pub struct McpUsage {
    pub connection_id: String,
    pub connection_name: String,
    pub owner_id: String,
    pub owner_name: String,
    pub client: String,
    pub calls: u64,
    pub errors: u64,
    pub last_used_ms: i64,
    pub last_success_ms: Option<i64>,
}

#[derive(Default, Serialize, JsonSchema)]
pub struct McpActivity {
    pub usage: Vec<McpUsage>,
    pub usage_truncated: bool,
    pub calls: Vec<McpCall>,
    pub next_cursor: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct McpOverview {
    pub connections: Vec<McpConnection>,
    pub activity: McpActivity,
    pub authentication: McpAuthentication,
    pub can_create: bool,
    pub can_manage_all: bool,
    pub scope: String,
    pub gateway_url: Option<String>,
    pub from_ms: i64,
    pub to_ms: i64,
    pub retention_days: u16,
}
