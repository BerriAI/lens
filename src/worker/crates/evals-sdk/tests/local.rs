use lens_evals_sdk::{
    Error,
    local::{AgentResult, Attempt, Outcome, Progress, RunMeta, Target, Task, ToolCall, run::*},
};
use rstest::rstest;
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;

fn target(model: &str) -> Target {
    Target {
        harness: "custom".to_owned(),
        model: model.to_owned(),
    }
}

fn attempt(
    model: &str,
    outcome: Outcome,
    latency: f64,
    cost: f64,
    calls: Vec<ToolCall>,
) -> Attempt {
    Attempt {
        task_id: "t".to_owned(),
        target: target(model),
        latency_seconds: latency,
        result: AgentResult {
            outcome,
            output: String::new(),
            cost,
            judge_cost: 0.0,
            tool_calls: calls,
        },
    }
}

fn call(name: &str, arguments: serde_json::Value, error: bool) -> ToolCall {
    ToolCall {
        name: name.to_owned(),
        arguments,
        error,
    }
}

fn fail() -> Outcome {
    Outcome::Fail {
        reason: "wrong".to_owned(),
    }
}

fn meta(models: &[&str]) -> RunMeta {
    RunMeta {
        id: "eval_test".to_owned(),
        tasks_file: "tasks.csv".to_owned(),
        config_file: "evals.py".to_owned(),
        targets: models.iter().map(|model| target(model)).collect(),
        task_count: 1,
        concurrency: 2,
    }
}

#[rstest]
fn load_tasks_keeps_extra_columns_as_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tasks.csv");
    fs::write(
        &path,
        "id,prompt,expected,tier\na,hi there,billing,gold\nb,yo,,free\n",
    )
    .unwrap();
    let tasks = load_tasks(&path).unwrap();
    assert_eq!(tasks[0].expected.as_deref(), Some("billing"));
    assert_eq!(
        tasks[0].metadata,
        BTreeMap::from([("tier".to_owned(), "gold".to_owned())])
    );
    assert_eq!(tasks[1].expected, None);
}

#[rstest]
#[case::no_id("prompt\nhello\n", "id")]
#[case::no_prompt("id\na\n", "prompt")]
fn load_tasks_rejects_missing_columns(#[case] contents: &str, #[case] column: &str) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tasks.csv");
    fs::write(&path, contents).unwrap();
    match load_tasks(&path) {
        Err(Error::Invalid { value, .. }) => assert_eq!(value, column),
        other => panic!("expected missing column, got {other:?}"),
    }
}

#[rstest]
#[case::empty("id,prompt\n,hello\n", "")]
#[case::blank("id,prompt\n  ,hello\n", "  ")]
#[case::duplicate("id,prompt\na,hello\na,world\n", "a")]
fn load_tasks_rejects_ambiguous_checkpoint_ids(#[case] contents: &str, #[case] invalid: &str) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tasks.csv");
    fs::write(&path, contents).unwrap();
    match load_tasks(&path) {
        Err(Error::Invalid { value, .. }) => assert_eq!(value, invalid),
        other => panic!("expected invalid task id, got {other:?}"),
    }
}

#[rstest]
fn run_id_has_eval_prefix_and_random_uuid() {
    let id = new_run_id();
    let suffix = id.strip_prefix("eval_").expect("run prefix");
    let uuid = uuid::Uuid::parse_str(suffix).expect("random UUID");
    assert_eq!(uuid.get_version(), Some(uuid::Version::Random));
}

#[rstest]
fn summarize_classifies_failures_by_tool_errors() {
    let attempts = vec![
        attempt("m", Outcome::Pass, 1.0, 0.5, vec![]),
        attempt("m", fail(), 2.0, 0.5, vec![call("lookup", json!({}), true)]),
        attempt(
            "m",
            fail(),
            3.0,
            0.5,
            vec![call("lookup", json!({}), false)],
        ),
        attempt(
            "m",
            Outcome::Error {
                kind: "Timeout".to_owned(),
                message: "slow".to_owned(),
            },
            4.0,
            0.5,
            vec![],
        ),
    ];
    let stat = &summarize(&meta(&["m"]), &attempts).stats[0];
    assert_eq!((stat.pass, stat.fail, stat.error), (1, 2, 1));
    assert_eq!((stat.wrong_answer, stat.tool_misuse), (1, 1));
    assert!((stat.model_cost - 2.0).abs() < 1e-9);
    assert!((stat.tool_calls_per_case - 0.5).abs() < 1e-9);
}

#[rstest]
fn repeat_rate_ignores_argument_key_order() {
    let calls = vec![
        call("lookup", json!({"ticket": 1, "source": "a"}), false),
        call("lookup", json!({"source": "a", "ticket": 1}), false),
        call("lookup", json!({"ticket": 2, "source": "a"}), false),
        call("reply", json!({"ticket": 1, "source": "a"}), false),
    ];
    let stat = &summarize(
        &meta(&["m"]),
        &[attempt("m", Outcome::Pass, 1.0, 0.0, calls)],
    )
    .stats[0];
    assert!((stat.repeat_rate - 0.25).abs() < 1e-9);
}

fn recommendation_attempts(rows: &[(u32, f64)]) -> Vec<Attempt> {
    let models = ["a", "b"];
    rows.iter()
        .zip(models)
        .flat_map(|(&(passes, cost), model)| {
            (0..20).map(move |index| {
                let outcome = if index < passes {
                    Outcome::Pass
                } else {
                    fail()
                };
                attempt(model, outcome, 1.0, cost / 20.0, vec![])
            })
        })
        .collect()
}

#[rstest]
#[case::cheaper_within_window(&[(19, 10.0), (18, 2.0)], Some(1))]
#[case::cheaper_outside_window(&[(19, 10.0), (15, 2.0)], Some(0))]
#[case::no_passes(&[(0, 1.0), (0, 2.0)], None)]
fn recommendation_picks_cheapest_pass_near_the_best(
    #[case] rows: &[(u32, f64)],
    #[case] expected: Option<usize>,
) {
    let models = ["a", "b"];
    let attempts = recommendation_attempts(rows);
    assert_eq!(summarize(&meta(&models), &attempts).recommended, expected);
}

fn answer() -> AgentResult {
    AgentResult {
        outcome: Outcome::Pass,
        output: "ok".to_owned(),
        cost: 0.01,
        judge_cost: 0.0,
        tool_calls: vec![],
    }
}

fn tasks(ids: &[&str]) -> Vec<Task> {
    ids.iter()
        .map(|id| Task {
            id: (*id).to_owned(),
            prompt: "p".to_owned(),
            expected: None,
            metadata: BTreeMap::new(),
        })
        .collect()
}

#[rstest]
#[tokio::test]
async fn execute_journals_every_attempt_and_resumes() {
    let directory = tempfile::tempdir().unwrap();
    let run = RunMeta {
        task_count: 2,
        ..meta(&["a", "b"])
    };
    let mut first: Vec<Progress> = Vec::new();
    let summary = execute(
        directory.path(),
        RunMeta {
            task_count: 1,
            ..run.clone()
        },
        tasks(&["one"]),
        |_, _| async { answer() },
        &mut |event| first.push(event),
    )
    .await
    .unwrap();
    assert_eq!(summary.stats.iter().map(|stat| stat.pass).sum::<u32>(), 2);
    assert!(matches!(first[0], Progress::Started { resumed: 0, .. }));
    assert_eq!(first.len(), 3);

    let calls = std::sync::atomic::AtomicUsize::new(0);
    let mut second: Vec<Progress> = Vec::new();
    let summary = execute(
        directory.path(),
        run.clone(),
        tasks(&["one", "two"]),
        |task, _| {
            assert_eq!(task.id, "two");
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async { answer() }
        },
        &mut |event| second.push(event),
    )
    .await
    .unwrap();
    assert!(matches!(second[0], Progress::Started { resumed: 2, .. }));
    assert_eq!(calls.into_inner(), 2);
    assert_eq!(summary.stats.iter().map(|stat| stat.pass).sum::<u32>(), 4);
    let (loaded, attempts) = load_run(directory.path(), &run.id).unwrap();
    assert_eq!((loaded, attempts.len()), (run, 4));
    assert_eq!(latest_run(directory.path()).as_deref(), Some("eval_test"));
    assert!(
        directory
            .path()
            .join(".lens/runs/eval_test/summary.json")
            .exists()
    );
}

#[rstest]
fn saved_report_screen_has_the_report() {
    let quiet = lens_evals_sdk::demo::Options {
        animate: false,
        color: false,
        width: 100,
        height: 40,
    };
    let lines: Vec<String> = lens_evals_sdk::local::view::report_screen(
        &quiet,
        "lens show eval_7f2a",
        &lens_evals_sdk::demo::fixture::summary(),
    )
    .iter()
    .map(lens_evals_sdk::demo::style::Line::plain)
    .collect();
    assert!(
        lines
            .iter()
            .any(|line| line.contains("LENS REPORT  eval_7f2a"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("❯ lens show eval_7f2a"))
    );
    assert!(lines.iter().any(|line| line.contains("▍08  Next")));
}
