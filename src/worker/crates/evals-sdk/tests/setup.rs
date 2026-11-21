use lens_evals_sdk::setup::Settings;
use rstest::rstest;
use std::fs;
use tempfile::TempDir;

#[rstest]
fn local_connections_do_not_change_public_settings() {
    let root: TempDir = tempfile::tempdir().unwrap();
    fs::write(root.path().join("pyproject.toml"), "[tool.lens]\nproject='agent'\n[tool.lens.connections.local]\nauth='none'\nbase_url_env='AGENT_URL'\n").unwrap();
    let settings = Settings::load(root.path()).unwrap();
    assert!(settings.connections.contains_key("local"));
    assert_eq!(
        serde_json::to_value(settings).unwrap(),
        serde_json::json!({"project":"agent","evals":"evals/","base_url":""})
    );
}
