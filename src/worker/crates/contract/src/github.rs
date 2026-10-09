use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressState {
    Running,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgressRequest {
    pub name: String,
    pub version: String,
    pub pr: u64,
    pub ci_url: String,
    pub state: ProgressState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressPublished {
    pub comment_url: String,
    pub check_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repository {
    pub id: u64,
    pub full_name: String,
    pub installation_id: u64,
    pub default_branch: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connection {
    pub agent: String,
    pub repository_id: u64,
    pub repository: String,
    pub installation_id: u64,
    pub default_branch: String,
    pub connected_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owner {
    pub scope: String,
    pub subject: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AuthorizationState {
    Installing,
    Pending,
    Exchanging,
    Ready { repositories: Vec<Repository> },
    Failed { error: String },
    Connected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Authorization {
    pub id: String,
    pub owner: Owner,
    pub agent: String,
    pub expires_at: DateTime<Utc>,
    #[serde(flatten)]
    pub state: AuthorizationState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BrokerHandshakeState {
    Pending,
    Selected {
        code_hash: String,
        connection: Connection,
    },
    Redeemed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerHandshake {
    pub id: String,
    pub browser_claimed: bool,
    pub browser_hash: String,
    pub lens_origin: String,
    pub redirect_uri: String,
    pub local_state: String,
    pub code_challenge: String,
    pub agent: String,
    pub expires_at: DateTime<Utc>,
    #[serde(flatten)]
    pub state: BrokerHandshakeState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerConnection {
    pub id: String,
    pub lens_origin: String,
    pub connection: Connection,
    pub capability_hash: String,
    pub revoked: bool,
}
