use lens_cli::arguments;
use rstest::rstest;
use serde_json::json;

#[rstest]
#[case::named(vec!["lens", "eval", "--name", "saved"], 0)]
#[case::conflicting_path(vec!["lens", "eval", "evals/", "--name", "saved"], 2)]
#[case::conflicting_filter(vec!["lens", "eval", "--eval", "local", "--name", "saved"], 2)]
fn saved_name_is_distinct_from_local_discovery(#[case] arguments: Vec<&str>, #[case] code: u64) {
    let result = arguments::parse(&arguments.into_iter().map(str::to_owned).collect::<Vec<_>>());
    assert_eq!(
        result
            .get("code")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        code
    );
    if code == 0 {
        assert_eq!(result["eval_name"], "saved");
    }
}

#[rstest]
#[case::all(vec![], Vec::<&str>::new())]
#[case::one(vec!["--only", "a/x"], vec!["a/x"])]
#[case::repeated(vec!["--only", "a/x", "--only", "b/provider/model"], vec!["a/x", "b/provider/model"])]
fn run_preserves_target_filters(#[case] selection: Vec<&str>, #[case] expected: Vec<&str>) {
    let arguments = ["lens", "run", "tasks.csv", "--config", "evals.py"]
        .into_iter()
        .chain(selection)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let parsed = arguments::parse(&arguments);
    assert_eq!(parsed["command"], "run");
    assert_eq!(parsed["only"], serde_json::json!(expected));
}
#[rstest]
#[case::actions(vec!["lens", "report", "report.json"], false)]
#[case::app(vec!["lens", "report", "report.json", "--via-app"], true)]
fn reporting_mode(#[case] arguments: Vec<&str>, #[case] via_app: bool) {
    let parsed = arguments::parse(&arguments.into_iter().map(str::to_owned).collect::<Vec<_>>());
    assert_eq!(
        parsed,
        json!({"command":"report", "file":"report.json", "via_app": via_app})
    );
}

#[rstest]
#[case::starting("report-start")]
#[case::failed("report-failed")]
fn progress_commands_accept_a_dispatch_pull_request(#[case] command: &str) {
    let parsed = arguments::parse(&[
        "lens".into(),
        command.into(),
        "--name".into(),
        "demo".into(),
        "--pr".into(),
        "7".into(),
    ]);
    assert_eq!(parsed, json!({"command":command,"name":"demo","pr":7}));
}
