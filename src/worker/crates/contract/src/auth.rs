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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
pub enum Scope {
    #[serde(rename = "traces:read")]
    TracesRead,
    #[serde(rename = "datasets:read")]
    DatasetsRead,
    #[serde(rename = "datasets:write")]
    DatasetsWrite,
    #[serde(rename = "investigations:read")]
    InvestigationsRead,
    #[serde(rename = "investigations:write")]
    InvestigationsWrite,
    #[serde(rename = "models:read")]
    ModelsRead,
    #[serde(rename = "models:write")]
    ModelsWrite,
    #[serde(rename = "signals:read")]
    SignalsRead,
    #[serde(rename = "signals:write")]
    SignalsWrite,
    #[serde(rename = "activity:read")]
    ActivityRead,
    #[serde(rename = "feedback:read")]
    FeedbackRead,
    #[serde(rename = "feedback:write")]
    FeedbackWrite,
    #[serde(rename = "evals:read")]
    EvalsRead,
    #[serde(rename = "evals:write")]
    EvalsWrite,
    #[serde(rename = "github:read")]
    GithubRead,
    #[serde(rename = "github:write")]
    GithubWrite,
    #[serde(rename = "profiles:read")]
    ProfilesRead,
    #[serde(rename = "profiles:write")]
    ProfilesWrite,
}

impl Role {
    pub fn reads_all_rows(self) -> bool {
        matches!(self, Self::ProxyAdmin | Self::ProxyAdminViewer)
    }

    pub fn scopes(self) -> &'static [Scope] {
        use Scope::*;
        match self {
            Self::ProxyAdmin => &[
                TracesRead,
                DatasetsRead,
                DatasetsWrite,
                InvestigationsRead,
                InvestigationsWrite,
                ModelsRead,
                ModelsWrite,
                SignalsRead,
                SignalsWrite,
                ActivityRead,
                FeedbackRead,
                FeedbackWrite,
                EvalsRead,
                EvalsWrite,
                GithubRead,
                GithubWrite,
                ProfilesRead,
                ProfilesWrite,
            ],
            Self::ProxyAdminViewer => &[
                TracesRead,
                DatasetsRead,
                InvestigationsRead,
                ModelsRead,
                SignalsRead,
                ActivityRead,
                FeedbackRead,
                EvalsRead,
                GithubRead,
                ProfilesRead,
            ],
            Self::OrgAdmin => &[
                TracesRead,
                FeedbackWrite,
                EvalsRead,
                EvalsWrite,
                GithubRead,
                GithubWrite,
            ],
            Self::InternalUser => &[
                TracesRead,
                FeedbackWrite,
                EvalsRead,
                EvalsWrite,
                GithubRead,
                GithubWrite,
            ],
            Self::InternalUserViewer => &[TracesRead, EvalsRead, GithubRead],
            Self::Team => &[
                TracesRead,
                FeedbackWrite,
                EvalsRead,
                EvalsWrite,
                GithubRead,
                GithubWrite,
            ],
            Self::Customer => &[TracesRead, EvalsRead, EvalsWrite, GithubRead],
            Self::SlackInvestigator => &[
                TracesRead,
                FeedbackWrite,
                EvalsRead,
                EvalsWrite,
                GithubRead,
                GithubWrite,
                InvestigationsRead,
                InvestigationsWrite,
            ],
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    #[serde(default)]
    pub user_role: Role,
    pub user_id: Option<String>,
    pub team_id: Option<String>,
    pub org_id: Option<String>,
    pub token: Option<String>,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub log_team_ids: Vec<String>,
}

impl Identity {
    pub fn allows(&self, scope: Scope) -> bool {
        self.user_role.scopes().contains(&scope)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionView {
    pub user_id: String,
    pub user_role: Role,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionRequest {
    pub token: String,
}
