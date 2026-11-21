use lens_cli::{
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
        label: None,
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
        trial: 1,
        target: target(model),
        latency_seconds: latency,
        result: AgentResult {
            outcome,
            output: String::new(),
            cost: Some(cost),
            judge_cost: Some(0.0),
            tool_calls: calls,
            metrics: lens_cli::local::Metrics {
                metrics_complete: true,
                ..Default::default()
            },
            stop_reason: None,
            check_error: None,
            check_detail: None,
            duration_seconds: None,
            run_id: None,
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
        task_data: None,
        config_hash: None,
        targets: models.iter().map(|model| target(model)).collect(),
        task_count: 1,
        trials: 1,
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
fn run_id_is_short_hex_and_unique() {
    let id = new_run_id();
    let suffix = id.strip_prefix("eval_").expect("run prefix");
    assert_eq!(suffix.len(), 8);
    assert!(suffix.chars().all(|ch| ch.is_ascii_hexdigit()));
    assert_ne!(id, new_run_id());
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
    assert!((stat.model_cost.unwrap() - 2.0).abs() < 1e-9);
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
        cost: Some(0.01),
        judge_cost: Some(0.0),
        tool_calls: vec![],
        metrics: lens_cli::local::Metrics {
            metrics_complete: true,
            ..Default::default()
        },
        stop_reason: None,
        check_error: None,
        check_detail: None,
        duration_seconds: None,
        run_id: None,
    }
}

fn tasks(ids: &[&str]) -> Vec<Task> {
    ids.iter()
        .map(|id| Task {
            id: (*id).to_owned(),
            prompt: "p".to_owned(),
            expected: None,
            metadata: BTreeMap::new(),
            suite_id: None,
            trial: 1,
        })
        .collect()
}

#[rstest]
#[tokio::test]
async fn execute_journals_every_attempt_and_resumes() {
    let directory = tempfile::tempdir().unwrap();
    let run = RunMeta {
        task_count: 2,
        trials: 1,
        ..meta(&["a", "b"])
    };
    let mut first: Vec<Progress> = Vec::new();
    let summary = execute(
        directory.path(),
        RunMeta {
            task_count: 1,
            trials: 1,
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
    let quiet = lens_cli::demo::Options {
        animate: false,
        color: false,
        width: 100,
        height: 40,
    };
    let lines: Vec<String> = lens_cli::local::view::report_screen(
        &quiet,
        "lens show eval_7f2a",
        &lens_cli::demo::fixture::summary(),
    )
    .iter()
    .map(lens_cli::demo::style::Line::plain)
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

#[rstest]
#[tokio::test]
async fn resume_finishes_only_missing_trials() {
    let directory = tempfile::tempdir().unwrap();
    let run = RunMeta {
        trials: 3,
        ..meta(&["a"])
    };
    let checkpoint = directory.path().join(".lens/runs/eval_test");
    fs::create_dir_all(&checkpoint).unwrap();
    let first = attempt("a", Outcome::Pass, 1.0, 0.1, vec![]);
    fs::write(
        checkpoint.join("attempts.jsonl"),
        format!("{}\n", serde_json::to_string(&first).unwrap()),
    )
    .unwrap();
    let invoked = std::sync::Mutex::new(Vec::new());
    let mut events = Vec::new();
    let summary = execute(
        directory.path(),
        run.clone(),
        tasks(&["t"]),
        |task, _| {
            invoked.lock().unwrap().push(task.trial);
            async { answer() }
        },
        &mut |event| events.push(event),
    )
    .await
    .unwrap();
    let mut trials = invoked.into_inner().unwrap();
    trials.sort_unstable();
    assert_eq!(trials, [2, 3]);
    assert_eq!(summary.stats[0].pass, 3);
    assert_eq!(run.total_cases(), 3);
    assert!(matches!(events[0], Progress::Started { resumed: 1, .. }));
    let (_, attempts) = load_run(directory.path(), &run.id).unwrap();
    assert_eq!(
        attempts
            .iter()
            .map(|attempt| attempt.trial)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
}

#[rstest]
#[case::unchanged("p", Some("original"), true)]
#[case::task_changed("changed", Some("original"), false)]
#[case::config_changed("p", Some("changed"), false)]
#[case::hash_missing("p", None, false)]
fn resume_checks_task_and_config_identity(
    #[case] prompt: &str,
    #[case] config_hash: Option<&str>,
    #[case] accepted: bool,
) {
    let run = RunMeta {
        task_data: Some(tasks(&["t"])),
        config_hash: Some("original".to_owned()),
        ..meta(&["a"])
    };
    let mut current = tasks(&["t"]);
    current[0].prompt = prompt.to_owned();
    assert_eq!(
        validate_resume(&run, &current, &run.targets, config_hash).is_ok(),
        accepted
    );
}

#[rstest]
#[case::saved_selection_is_a_subset("a", "custom", Some("pick"), true)]
#[case::same_label_changed_model("changed", "custom", Some("pick"), false)]
#[case::same_label_changed_harness("a", "changed", Some("pick"), false)]
#[case::label_removed("a", "custom", None, false)]
fn resume_keeps_the_resolved_targets_from_the_original_selection(
    #[case] model: &str,
    #[case] harness: &str,
    #[case] label: Option<&str>,
    #[case] accepted: bool,
) {
    let run = RunMeta {
        targets: vec![Target {
            label: Some("pick".to_owned()),
            ..target("a")
        }],
        ..meta(&[])
    };
    let current_targets = vec![
        Target {
            harness: harness.to_owned(),
            model: model.to_owned(),
            label: label.map(str::to_owned),
        },
        target("additional"),
    ];
    assert_eq!(
        validate_resume(&run, &tasks(&["t"]), &current_targets, None).is_ok(),
        accepted
    );
}

#[rstest]
#[case::unknown_cost(None, true)]
#[case::incomplete_metrics(Some(0.0), false)]
fn incomplete_metrics_cannot_look_like_a_free_recommendation(
    #[case] cost: Option<f64>,
    #[case] complete: bool,
) {
    let mut row = attempt("a", Outcome::Pass, 1.0, 0.1, vec![]);
    row.result.cost = cost;
    row.result.metrics.metrics_complete = complete;
    row.result.metrics.input_tokens = Some(7);
    row.result.metrics.output_tokens = None;
    let summary = summarize(&meta(&["a"]), &[row]);
    assert_eq!(summary.recommended, None);
    assert_eq!(summary.stats[0].model_cost, cost);
    assert_eq!(summary.stats[0].metrics.input_tokens, Some(7));
    assert_eq!(summary.stats[0].metrics.output_tokens, None);
    let report = lens_cli::local::view::report(&summary, 100)
        .iter()
        .map(lens_cli::demo::style::Line::plain)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(report.contains("Cost comparison needs complete"));
}

#[rstest]
fn old_checkpoints_default_to_one_trial() {
    let run: RunMeta = serde_json::from_value(json!({"id":"old", "tasks_file":"tasks.csv", "config_file":"evals.py", "targets":[], "task_count":1, "concurrency":1})).unwrap();
    let row: Attempt = serde_json::from_value(json!({"task_id":"t", "target":{"harness":"custom","model":"a"}, "latency_seconds":1, "outcome":{"status":"pass"}, "cost":0.1,"judge_cost":0})).unwrap();
    assert_eq!(run.trials, 1);
    assert_eq!(row.trial, 1);
    assert_eq!(row.result.cost, Some(0.1));
    assert!(!row.result.metrics.metrics_complete);
}

#[rstest]
#[tokio::test]
async fn execute_preserves_sdk_attempt_identity_and_duration() {
    let directory = tempfile::tempdir().unwrap();
    let run = RunMeta {
        targets: vec![
            Target {
                label: Some("careful".to_owned()),
                ..target("same")
            },
            Target {
                label: Some("fast".to_owned()),
                ..target("same")
            },
        ],
        ..meta(&[])
    };
    let summary = execute(
        directory.path(),
        run.clone(),
        tasks(&["t"]),
        |task, target| {
            assert_eq!(task.suite_id.as_deref(), Some("eval_test"));
            async move {
                AgentResult {
                    duration_seconds: Some(0.5),
                    run_id: Some(target.key()),
                    ..answer()
                }
            }
        },
        &mut |_| {},
    )
    .await
    .unwrap();
    assert_eq!(summary.stats.len(), 2);
    assert_eq!(summary.stats[0].pass, 1);
    assert_eq!(summary.stats[1].pass, 1);
    assert_eq!(summary.stats[0].p50_seconds, 0.5);
    let (_, mut attempts) = load_run(directory.path(), &run.id).unwrap();
    attempts.sort_by_key(|attempt| attempt.target.key());
    assert_eq!(attempts[0].result.run_id.as_deref(), Some("careful"));
    assert_eq!(attempts[1].result.run_id.as_deref(), Some("fast"));
    let report = lens_cli::local::view::report(&summary, 100)
        .iter()
        .map(lens_cli::demo::style::Line::plain)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(report.contains("careful"));
    assert!(report.contains("fast"));
}
