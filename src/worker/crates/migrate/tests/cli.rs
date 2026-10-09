use rstest::rstest;
use std::process::Command;

#[rstest]
fn help_requires_no_database_credentials() {
    let result = Command::new(env!("CARGO_BIN_EXE_lens-migrate"))
        .arg("--help")
        .env_clear()
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains("--apply --source-stopped")
    );
    assert!(result.stderr.is_empty());
}

#[rstest]
#[case::no_credential(vec![])]
#[case::missing_stop_assertion(vec!["--apply"])]
#[case::unknown_option(vec!["--delete"])]
fn invalid_commands_fail_before_connecting(#[case] arguments: Vec<&str>) {
    let result = Command::new(env!("CARGO_BIN_EXE_lens-migrate"))
        .args(arguments)
        .env_clear()
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(
        String::from_utf8(result.stderr)
            .unwrap()
            .contains("configuration")
    );
}
