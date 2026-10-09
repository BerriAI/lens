use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
