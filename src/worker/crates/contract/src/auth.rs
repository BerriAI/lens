use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    ProxyAdmin,
    ProxyAdminViewer,
    OrgAdmin,
    #[default]
    InternalUser,
    InternalUserViewer,
    Team,
    Customer,
    #[serde(skip)]
    #[schemars(skip)]
    SlackInvestigator,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    #[serde(default)]
    pub user_role: Role,
    pub user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
    pub team_id: Option<String>,
    pub org_id: Option<String>,
    pub token: Option<String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub log_team_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionView {
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
    pub user_role: Role,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionRequest {
    pub token: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SignInMethods {
    pub litellm_login_url: Option<String>,
}
