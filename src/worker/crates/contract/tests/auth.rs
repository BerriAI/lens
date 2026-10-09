use lens_contract::auth::{Identity, Role, SessionRequest, SessionView};
use rstest::rstest;
use serde_json::{Value, json};

#[rstest]
#[case::admin("proxy_admin", Role::ProxyAdmin)]
#[case::admin_viewer("proxy_admin_viewer", Role::ProxyAdminViewer)]
#[case::org_admin("org_admin", Role::OrgAdmin)]
#[case::user("internal_user", Role::InternalUser)]
#[case::user_viewer("internal_user_viewer", Role::InternalUserViewer)]
#[case::team("team", Role::Team)]
#[case::customer("customer", Role::Customer)]
fn role_wire_values(#[case] wire: &str, #[case] role: Role) {
    assert_eq!(serde_json::to_value(role).unwrap(), json!(wire));
    assert_eq!(serde_json::from_value::<Role>(json!(wire)).unwrap(), role);
}

#[rstest]
fn identity_defaults_match_gateway_contract() {
    let identity: Identity = serde_json::from_value(json!({})).unwrap();
    assert_eq!(identity, Identity::default());
    assert_eq!(
        serde_json::to_value(identity).unwrap(),
        json!({"user_role":"internal_user", "user_id":null, "team_id":null, "org_id":null, "token":null, "models":[], "log_team_ids":[]})
    );
}

#[rstest]
#[case::unknown_field(json!({"unknown": true}))]
#[case::unknown_role(json!({"user_role": "owner"}))]
#[case::null_role(json!({"user_role": null}))]
#[case::numeric_id(json!({"user_id": 12}))]
#[case::non_string_model(json!({"models": [12]}))]
#[case::null_teams(json!({"log_team_ids": null}))]
fn identity_rejects_invalid_scope(#[case] value: Value) {
    assert!(serde_json::from_value::<Identity>(value).is_err());
}

#[rstest]
fn session_contract_retains_exact_response() {
    let view = SessionView {
        user_id: "user".into(),
        user_role: Role::Team,
    };
    let value = json!({"user_id":"user", "user_role":"team"});
    assert_eq!(serde_json::to_value(&view).unwrap(), value);
    assert_eq!(serde_json::from_value::<SessionView>(value).unwrap(), view);
    assert_eq!(
        serde_json::from_value::<SessionRequest>(json!({"token":"setup"}))
            .unwrap()
            .token,
        "setup"
    );
    assert!(serde_json::from_value::<SessionRequest>(json!({"token":"setup", "extra":1})).is_err());
}
