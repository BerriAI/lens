use lens_evals_sdk::setup::{self, InitOptions, Settings};
use rstest::{fixture, rstest};
use std::fs;
use tempfile::TempDir;

#[fixture]
fn root() -> TempDir {
    tempfile::tempdir().unwrap()
}
#[fixture]
fn options() -> InitOptions {
    InitOptions {
        dataset: Some("demo@1".into()),
        project: Some("demo".into()),
        base_url: Some("http://127.0.0.1:8765".into()),
        task: None,
        demo: false,
        action: "BerriAI/lens/src/sdk/action@preview".into(),
    }
}

#[rstest]
fn guided_setup_preserves_existing_configuration(root: TempDir, options: InitOptions) {
    fs::write(
        root.path().join("pyproject.toml"),
        "# keep my comment\n[project]\nname='agent'\n[tool.lens]\nevals='checks/'\n",
    )
    .unwrap();
    let mut values = ["demo@1", "demo", "http://127.0.0.1:8765", "agent:run"].into_iter();
    let options = InitOptions {
        dataset: None,
        project: None,
        base_url: None,
        ..options
    };
    setup::initialize_with(root.path(), options, true, &mut |_| {
        Ok(values.next().unwrap().into())
    })
    .unwrap();
    let config = Settings::load(root.path()).unwrap();
    assert_eq!(config.project, "demo");
    assert_eq!(config.evals, "checks/");
    assert!(
        fs::read_to_string(root.path().join("pyproject.toml"))
            .unwrap()
            .contains("# keep my comment")
    );
    assert!(root.path().join("checks/demo.py").exists());
}

#[rstest]
#[case::eval("evals/demo.py")]
#[case::workflow(".github/workflows/lens.yml")]
fn existing_files_are_untouched(root: TempDir, options: InitOptions, #[case] existing: &str) {
    let path = root.path().join(existing);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "keep me").unwrap();
    assert!(setup::initialize_with(root.path(), options, false, &mut |_| unreachable!()).is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), "keep me");
    assert!(!root.path().join("pyproject.toml").exists());
}

#[rstest]
#[case::invalid_dataset("Bad name", "agent:run")]
#[case::invalid_task("demo", "agent;print:run")]
fn rejects_invalid_input_before_writing(
    root: TempDir,
    options: InitOptions,
    #[case] dataset: &str,
    #[case] task: &str,
) {
    let options = InitOptions {
        dataset: Some(dataset.into()),
        task: Some(task.into()),
        ..options
    };
    assert!(setup::initialize_with(root.path(), options, false, &mut |_| unreachable!()).is_err());
    assert!(!root.path().join("pyproject.toml").exists());
}

#[rstest]
fn demo_has_no_ci_gate(root: TempDir, options: InitOptions) {
    let options = InitOptions {
        demo: true,
        ..options
    };
    setup::initialize_with(root.path(), options, false, &mut |_| unreachable!()).unwrap();
    assert!(root.path().join("evals/demo.py").exists());
    assert!(!root.path().join(".github/workflows/lens.yml").exists());
}

#[rstest]
#[case::scalar_tool("tool = 'text'")]
#[case::scalar_lens("[tool]\nlens = 7")]
#[case::malformed("[tool.lens")]
fn malformed_configuration_has_no_partial_writes(
    root: TempDir,
    options: InitOptions,
    #[case] config: &str,
) {
    fs::write(root.path().join("pyproject.toml"), config).unwrap();
    assert!(setup::initialize_with(root.path(), options, false, &mut |_| unreachable!()).is_err());
    assert_eq!(
        fs::read_to_string(root.path().join("pyproject.toml")).unwrap(),
        config
    );
    assert!(!root.path().join("evals").exists());
}

#[rstest]
#[case::keyword("agent:lambda")]
#[case::keyword_module("agent.with:run")]
fn keyword_imports_are_rejected(root: TempDir, options: InitOptions, #[case] task: &str) {
    assert!(
        setup::initialize_with(
            root.path(),
            InitOptions {
                task: Some(task.into()),
                ..options
            },
            false,
            &mut |_| unreachable!()
        )
        .is_err()
    );
    assert!(!root.path().join("pyproject.toml").exists());
}

#[rstest]
fn uv_workflow_uses_agent_environment(root: TempDir, options: InitOptions) {
    fs::write(root.path().join("uv.lock"), "").unwrap();
    setup::initialize_with(root.path(), options, false, &mut |_| unreachable!()).unwrap();
    let workflow = fs::read_to_string(root.path().join(".github/workflows/lens.yml")).unwrap();
    assert!(workflow.contains("uv sync --frozen"));
    assert!(workflow.contains("python: .venv/bin/python"));
}
