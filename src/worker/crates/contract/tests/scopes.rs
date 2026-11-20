use lens_contract::auth::{Identity, Role, Scope};
use rstest::rstest;

#[rstest]
#[case::admin(Role::ProxyAdmin, true)]
#[case::admin_viewer(Role::ProxyAdminViewer, true)]
#[case::org_admin(Role::OrgAdmin, false)]
#[case::user(Role::InternalUser, false)]
#[case::user_viewer(Role::InternalUserViewer, false)]
#[case::team(Role::Team, false)]
#[case::customer(Role::Customer, false)]
#[case::slack_investigator(Role::SlackInvestigator, false)]
fn role_row_access(#[case] role: Role, #[case] all_rows: bool) {
    assert_eq!(role.reads_all_rows(), all_rows);
}

#[rstest]
#[case::traces_read(Scope::TracesRead, "traces:read")]
#[case::datasets_read(Scope::DatasetsRead, "datasets:read")]
#[case::datasets_write(Scope::DatasetsWrite, "datasets:write")]
#[case::investigations_read(Scope::InvestigationsRead, "investigations:read")]
#[case::investigations_write(Scope::InvestigationsWrite, "investigations:write")]
#[case::models_read(Scope::ModelsRead, "models:read")]
#[case::models_write(Scope::ModelsWrite, "models:write")]
#[case::signals_read(Scope::SignalsRead, "signals:read")]
#[case::signals_write(Scope::SignalsWrite, "signals:write")]
#[case::activity_read(Scope::ActivityRead, "activity:read")]
#[case::feedback_read(Scope::FeedbackRead, "feedback:read")]
#[case::feedback_write(Scope::FeedbackWrite, "feedback:write")]
#[case::evals_read(Scope::EvalsRead, "evals:read")]
#[case::evals_write(Scope::EvalsWrite, "evals:write")]
#[case::github_read(Scope::GithubRead, "github:read")]
#[case::github_write(Scope::GithubWrite, "github:write")]
#[case::profiles_read(Scope::ProfilesRead, "profiles:read")]
#[case::profiles_write(Scope::ProfilesWrite, "profiles:write")]
fn scope_wire_values(#[case] scope: Scope, #[case] wire: &str) {
    assert_eq!(
        serde_json::to_value(scope).unwrap(),
        serde_json::json!(wire)
    );
    assert_eq!(
        serde_json::from_value::<Scope>(serde_json::json!(wire)).unwrap(),
        scope
    );
}

#[rstest]
#[case::unknown("traces:delete")]
#[case::wrong_separator("traces_read")]
#[case::empty("")]
fn invalid_scope_wire_values(#[case] wire: &str) {
    assert!(serde_json::from_value::<Scope>(serde_json::json!(wire)).is_err());
}

#[rstest]
#[case::proxy_admin_traces_read(Role::ProxyAdmin, Scope::TracesRead, true)]
#[case::proxy_admin_datasets_read(Role::ProxyAdmin, Scope::DatasetsRead, true)]
#[case::proxy_admin_datasets_write(Role::ProxyAdmin, Scope::DatasetsWrite, true)]
#[case::proxy_admin_investigations_read(Role::ProxyAdmin, Scope::InvestigationsRead, true)]
#[case::proxy_admin_investigations_write(Role::ProxyAdmin, Scope::InvestigationsWrite, true)]
#[case::proxy_admin_models_read(Role::ProxyAdmin, Scope::ModelsRead, true)]
#[case::proxy_admin_models_write(Role::ProxyAdmin, Scope::ModelsWrite, true)]
#[case::proxy_admin_signals_read(Role::ProxyAdmin, Scope::SignalsRead, true)]
#[case::proxy_admin_signals_write(Role::ProxyAdmin, Scope::SignalsWrite, true)]
#[case::proxy_admin_activity_read(Role::ProxyAdmin, Scope::ActivityRead, true)]
#[case::proxy_admin_feedback_read(Role::ProxyAdmin, Scope::FeedbackRead, true)]
#[case::proxy_admin_feedback_write(Role::ProxyAdmin, Scope::FeedbackWrite, true)]
#[case::proxy_admin_evals_read(Role::ProxyAdmin, Scope::EvalsRead, true)]
#[case::proxy_admin_evals_write(Role::ProxyAdmin, Scope::EvalsWrite, true)]
#[case::proxy_admin_github_read(Role::ProxyAdmin, Scope::GithubRead, true)]
#[case::proxy_admin_github_write(Role::ProxyAdmin, Scope::GithubWrite, true)]
#[case::proxy_admin_profiles_read(Role::ProxyAdmin, Scope::ProfilesRead, true)]
#[case::proxy_admin_profiles_write(Role::ProxyAdmin, Scope::ProfilesWrite, true)]
#[case::proxy_admin_viewer_traces_read(Role::ProxyAdminViewer, Scope::TracesRead, true)]
#[case::proxy_admin_viewer_datasets_read(Role::ProxyAdminViewer, Scope::DatasetsRead, true)]
#[case::proxy_admin_viewer_datasets_write(Role::ProxyAdminViewer, Scope::DatasetsWrite, false)]
#[case::proxy_admin_viewer_investigations_read(
    Role::ProxyAdminViewer,
    Scope::InvestigationsRead,
    true
)]
#[case::proxy_admin_viewer_investigations_write(
    Role::ProxyAdminViewer,
    Scope::InvestigationsWrite,
    false
)]
#[case::proxy_admin_viewer_models_read(Role::ProxyAdminViewer, Scope::ModelsRead, true)]
#[case::proxy_admin_viewer_models_write(Role::ProxyAdminViewer, Scope::ModelsWrite, false)]
#[case::proxy_admin_viewer_signals_read(Role::ProxyAdminViewer, Scope::SignalsRead, true)]
#[case::proxy_admin_viewer_signals_write(Role::ProxyAdminViewer, Scope::SignalsWrite, false)]
#[case::proxy_admin_viewer_activity_read(Role::ProxyAdminViewer, Scope::ActivityRead, true)]
#[case::proxy_admin_viewer_feedback_read(Role::ProxyAdminViewer, Scope::FeedbackRead, true)]
#[case::proxy_admin_viewer_feedback_write(Role::ProxyAdminViewer, Scope::FeedbackWrite, false)]
#[case::proxy_admin_viewer_evals_read(Role::ProxyAdminViewer, Scope::EvalsRead, true)]
#[case::proxy_admin_viewer_evals_write(Role::ProxyAdminViewer, Scope::EvalsWrite, false)]
#[case::proxy_admin_viewer_github_read(Role::ProxyAdminViewer, Scope::GithubRead, true)]
#[case::proxy_admin_viewer_github_write(Role::ProxyAdminViewer, Scope::GithubWrite, false)]
#[case::proxy_admin_viewer_profiles_read(Role::ProxyAdminViewer, Scope::ProfilesRead, true)]
#[case::proxy_admin_viewer_profiles_write(Role::ProxyAdminViewer, Scope::ProfilesWrite, false)]
#[case::org_admin_traces_read(Role::OrgAdmin, Scope::TracesRead, true)]
#[case::org_admin_datasets_read(Role::OrgAdmin, Scope::DatasetsRead, false)]
#[case::org_admin_datasets_write(Role::OrgAdmin, Scope::DatasetsWrite, false)]
#[case::org_admin_investigations_read(Role::OrgAdmin, Scope::InvestigationsRead, false)]
#[case::org_admin_investigations_write(Role::OrgAdmin, Scope::InvestigationsWrite, false)]
#[case::org_admin_models_read(Role::OrgAdmin, Scope::ModelsRead, false)]
#[case::org_admin_models_write(Role::OrgAdmin, Scope::ModelsWrite, false)]
#[case::org_admin_signals_read(Role::OrgAdmin, Scope::SignalsRead, false)]
#[case::org_admin_signals_write(Role::OrgAdmin, Scope::SignalsWrite, false)]
#[case::org_admin_activity_read(Role::OrgAdmin, Scope::ActivityRead, false)]
#[case::org_admin_feedback_read(Role::OrgAdmin, Scope::FeedbackRead, false)]
#[case::org_admin_feedback_write(Role::OrgAdmin, Scope::FeedbackWrite, true)]
#[case::org_admin_evals_read(Role::OrgAdmin, Scope::EvalsRead, true)]
#[case::org_admin_evals_write(Role::OrgAdmin, Scope::EvalsWrite, true)]
#[case::org_admin_github_read(Role::OrgAdmin, Scope::GithubRead, true)]
#[case::org_admin_github_write(Role::OrgAdmin, Scope::GithubWrite, true)]
#[case::org_admin_profiles_read(Role::OrgAdmin, Scope::ProfilesRead, false)]
#[case::org_admin_profiles_write(Role::OrgAdmin, Scope::ProfilesWrite, false)]
#[case::internal_user_traces_read(Role::InternalUser, Scope::TracesRead, true)]
#[case::internal_user_datasets_read(Role::InternalUser, Scope::DatasetsRead, false)]
#[case::internal_user_datasets_write(Role::InternalUser, Scope::DatasetsWrite, false)]
#[case::internal_user_investigations_read(Role::InternalUser, Scope::InvestigationsRead, false)]
#[case::internal_user_investigations_write(Role::InternalUser, Scope::InvestigationsWrite, false)]
#[case::internal_user_models_read(Role::InternalUser, Scope::ModelsRead, false)]
#[case::internal_user_models_write(Role::InternalUser, Scope::ModelsWrite, false)]
#[case::internal_user_signals_read(Role::InternalUser, Scope::SignalsRead, false)]
#[case::internal_user_signals_write(Role::InternalUser, Scope::SignalsWrite, false)]
#[case::internal_user_activity_read(Role::InternalUser, Scope::ActivityRead, false)]
#[case::internal_user_feedback_read(Role::InternalUser, Scope::FeedbackRead, false)]
#[case::internal_user_feedback_write(Role::InternalUser, Scope::FeedbackWrite, true)]
#[case::internal_user_evals_read(Role::InternalUser, Scope::EvalsRead, true)]
#[case::internal_user_evals_write(Role::InternalUser, Scope::EvalsWrite, true)]
#[case::internal_user_github_read(Role::InternalUser, Scope::GithubRead, true)]
#[case::internal_user_github_write(Role::InternalUser, Scope::GithubWrite, true)]
#[case::internal_user_profiles_read(Role::InternalUser, Scope::ProfilesRead, false)]
#[case::internal_user_profiles_write(Role::InternalUser, Scope::ProfilesWrite, false)]
#[case::internal_user_viewer_traces_read(Role::InternalUserViewer, Scope::TracesRead, true)]
#[case::internal_user_viewer_datasets_read(Role::InternalUserViewer, Scope::DatasetsRead, false)]
#[case::internal_user_viewer_datasets_write(Role::InternalUserViewer, Scope::DatasetsWrite, false)]
#[case::internal_user_viewer_investigations_read(
    Role::InternalUserViewer,
    Scope::InvestigationsRead,
    false
)]
#[case::internal_user_viewer_investigations_write(
    Role::InternalUserViewer,
    Scope::InvestigationsWrite,
    false
)]
#[case::internal_user_viewer_models_read(Role::InternalUserViewer, Scope::ModelsRead, false)]
#[case::internal_user_viewer_models_write(Role::InternalUserViewer, Scope::ModelsWrite, false)]
#[case::internal_user_viewer_signals_read(Role::InternalUserViewer, Scope::SignalsRead, false)]
#[case::internal_user_viewer_signals_write(Role::InternalUserViewer, Scope::SignalsWrite, false)]
#[case::internal_user_viewer_activity_read(Role::InternalUserViewer, Scope::ActivityRead, false)]
#[case::internal_user_viewer_feedback_read(Role::InternalUserViewer, Scope::FeedbackRead, false)]
#[case::internal_user_viewer_feedback_write(Role::InternalUserViewer, Scope::FeedbackWrite, false)]
#[case::internal_user_viewer_evals_read(Role::InternalUserViewer, Scope::EvalsRead, true)]
#[case::internal_user_viewer_evals_write(Role::InternalUserViewer, Scope::EvalsWrite, false)]
#[case::internal_user_viewer_github_read(Role::InternalUserViewer, Scope::GithubRead, true)]
#[case::internal_user_viewer_github_write(Role::InternalUserViewer, Scope::GithubWrite, false)]
#[case::internal_user_viewer_profiles_read(Role::InternalUserViewer, Scope::ProfilesRead, false)]
#[case::internal_user_viewer_profiles_write(Role::InternalUserViewer, Scope::ProfilesWrite, false)]
#[case::team_traces_read(Role::Team, Scope::TracesRead, true)]
#[case::team_datasets_read(Role::Team, Scope::DatasetsRead, false)]
#[case::team_datasets_write(Role::Team, Scope::DatasetsWrite, false)]
#[case::team_investigations_read(Role::Team, Scope::InvestigationsRead, false)]
#[case::team_investigations_write(Role::Team, Scope::InvestigationsWrite, false)]
#[case::team_models_read(Role::Team, Scope::ModelsRead, false)]
#[case::team_models_write(Role::Team, Scope::ModelsWrite, false)]
#[case::team_signals_read(Role::Team, Scope::SignalsRead, false)]
#[case::team_signals_write(Role::Team, Scope::SignalsWrite, false)]
#[case::team_activity_read(Role::Team, Scope::ActivityRead, false)]
#[case::team_feedback_read(Role::Team, Scope::FeedbackRead, false)]
#[case::team_feedback_write(Role::Team, Scope::FeedbackWrite, true)]
#[case::team_evals_read(Role::Team, Scope::EvalsRead, true)]
#[case::team_evals_write(Role::Team, Scope::EvalsWrite, true)]
#[case::team_github_read(Role::Team, Scope::GithubRead, true)]
#[case::team_github_write(Role::Team, Scope::GithubWrite, true)]
#[case::team_profiles_read(Role::Team, Scope::ProfilesRead, false)]
#[case::team_profiles_write(Role::Team, Scope::ProfilesWrite, false)]
#[case::customer_traces_read(Role::Customer, Scope::TracesRead, true)]
#[case::customer_datasets_read(Role::Customer, Scope::DatasetsRead, false)]
#[case::customer_datasets_write(Role::Customer, Scope::DatasetsWrite, false)]
#[case::customer_investigations_read(Role::Customer, Scope::InvestigationsRead, false)]
#[case::customer_investigations_write(Role::Customer, Scope::InvestigationsWrite, false)]
#[case::customer_models_read(Role::Customer, Scope::ModelsRead, false)]
#[case::customer_models_write(Role::Customer, Scope::ModelsWrite, false)]
#[case::customer_signals_read(Role::Customer, Scope::SignalsRead, false)]
#[case::customer_signals_write(Role::Customer, Scope::SignalsWrite, false)]
#[case::customer_activity_read(Role::Customer, Scope::ActivityRead, false)]
#[case::customer_feedback_read(Role::Customer, Scope::FeedbackRead, false)]
#[case::customer_feedback_write(Role::Customer, Scope::FeedbackWrite, false)]
#[case::customer_evals_read(Role::Customer, Scope::EvalsRead, true)]
#[case::customer_evals_write(Role::Customer, Scope::EvalsWrite, true)]
#[case::customer_github_read(Role::Customer, Scope::GithubRead, true)]
#[case::customer_github_write(Role::Customer, Scope::GithubWrite, false)]
#[case::customer_profiles_read(Role::Customer, Scope::ProfilesRead, false)]
#[case::customer_profiles_write(Role::Customer, Scope::ProfilesWrite, false)]
#[case::slack_investigator_traces_read(Role::SlackInvestigator, Scope::TracesRead, true)]
#[case::slack_investigator_datasets_read(Role::SlackInvestigator, Scope::DatasetsRead, false)]
#[case::slack_investigator_datasets_write(Role::SlackInvestigator, Scope::DatasetsWrite, false)]
#[case::slack_investigator_investigations_read(
    Role::SlackInvestigator,
    Scope::InvestigationsRead,
    true
)]
#[case::slack_investigator_investigations_write(
    Role::SlackInvestigator,
    Scope::InvestigationsWrite,
    true
)]
#[case::slack_investigator_models_read(Role::SlackInvestigator, Scope::ModelsRead, false)]
#[case::slack_investigator_models_write(Role::SlackInvestigator, Scope::ModelsWrite, false)]
#[case::slack_investigator_signals_read(Role::SlackInvestigator, Scope::SignalsRead, false)]
#[case::slack_investigator_signals_write(Role::SlackInvestigator, Scope::SignalsWrite, false)]
#[case::slack_investigator_activity_read(Role::SlackInvestigator, Scope::ActivityRead, false)]
#[case::slack_investigator_feedback_read(Role::SlackInvestigator, Scope::FeedbackRead, false)]
#[case::slack_investigator_feedback_write(Role::SlackInvestigator, Scope::FeedbackWrite, true)]
#[case::slack_investigator_evals_read(Role::SlackInvestigator, Scope::EvalsRead, true)]
#[case::slack_investigator_evals_write(Role::SlackInvestigator, Scope::EvalsWrite, true)]
#[case::slack_investigator_github_read(Role::SlackInvestigator, Scope::GithubRead, true)]
#[case::slack_investigator_github_write(Role::SlackInvestigator, Scope::GithubWrite, true)]
#[case::slack_investigator_profiles_read(Role::SlackInvestigator, Scope::ProfilesRead, false)]
#[case::slack_investigator_profiles_write(Role::SlackInvestigator, Scope::ProfilesWrite, false)]
fn default_role_scopes(#[case] role: Role, #[case] scope: Scope, #[case] allowed: bool) {
    let identity = Identity {
        user_role: role,
        ..Identity::default()
    };
    assert_eq!(identity.allows(scope), allowed);
    assert_eq!(role.scopes().contains(&scope), allowed);
}
