mod support;

use lens_contract::eval::{Gate, Summary};
use lens_evals::{Baseline, CaseInput, RunInput, evaluate, is_critical};
use lens_evals_sdk::{client::Client, devserver, engine, model as sdk};
use rstest::rstest;
use serde_json::Value;
use support::{FakeJudge, case, error, run};

const CASES: usize = 36;
const TRIALS: usize = 3;

fn fixture(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../sdk/tests/fixtures/lens_eval")
        .join(format!("{name}.json"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn passing() -> lens_evals::Trial {
    lens_evals::Trial {
        cost_usd: Some(0.125),
        ..support::pass()
    }
}

fn cases(failing: Option<&str>) -> Vec<CaseInput> {
    devserver::sample_cases(CASES)
        .cases
        .iter()
        .map(|dataset_case| {
            let trial = if failing == Some(dataset_case.id.as_str()) {
                error()
            } else {
                passing()
            };
            CaseInput {
                title: dataset_case.id.clone(),
                ..case(
                    &dataset_case.id,
                    is_critical(&dataset_case.meta),
                    vec![trial; TRIALS],
                )
            }
        })
        .collect()
}

fn all_passing(run_id: &str, version: &str, url: &str) -> Baseline {
    Baseline {
        run_id: run_id.into(),
        version: version.into(),
        url: url.into(),
        verdicts: (0..CASES)
            .map(|index| (format!("case-{index}"), true))
            .collect(),
    }
}

async fn ours(input: RunInput) -> Value {
    let summary = evaluate(&input, &FakeJudge::constant(1.0))
        .await
        .unwrap()
        .summary;
    serde_json::to_value(summary).unwrap()
}

#[rstest]
#[case::done(
    "eval_run_done",
    Some(all_passing("baseline", "base", "http://localhost:8765/runs/baseline"))
)]
#[case::no_baseline("eval_run_no_baseline", None)]
#[tokio::test]
async fn summary_matches_golden_fixture(#[case] name: &str, #[case] baseline: Option<Baseline>) {
    let expected = fixture(name)["summary"].clone();
    let input = run(TRIALS, cases(None), baseline);
    assert_eq!(ours(input).await, expected);
    serde_json::from_value::<Summary>(expected).unwrap();
}

struct Server {
    client: Client,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve() -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let app = devserver::router(devserver::sample_cases(CASES), "lens-dev".into());
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Server {
        client: Client::new(&address, "lens-dev").unwrap(),
        task,
    }
}

fn spec() -> sdk::EvalSpec {
    sdk::EvalSpec {
        name: "demo".into(),
        data: "demo@1".into(),
        scores: vec![sdk::Scorer::TaskCompleted],
        baseline: "main".into(),
        trials: TRIALS,
        gate: sdk::Gate {
            pass_rate: Some(1.0),
            ..sdk::Gate::default()
        },
        concurrency: 8,
        timeout_seconds: 5.0,
        finding: None,
        case_ids: None,
    }
}

fn execution(branch: &str, identity: &str) -> sdk::Execution {
    sdk::Execution {
        version: identity.into(),
        branch: branch.into(),
        pr: (branch != "main").then_some(7),
        ci_url: String::new(),
        identity: identity.into(),
    }
}

async fn dev_run(
    server: &Server,
    branch: &str,
    identity: &str,
    failing: Option<&str>,
) -> sdk::Report {
    let failing = failing.map(str::to_owned);
    engine::evaluate(
        &server.client,
        &spec(),
        "demo",
        &execution(branch, identity),
        |case| {
            let failing = failing.clone();
            async move {
                if failing.as_deref() == Some(case.id.as_str()) {
                    engine::failure("ValueError", "agent failed")
                } else {
                    sdk::CaseResult {
                        trace: Some(sdk::TraceRef {
                            attribute: "session.id".into(),
                            value: "pass".into(),
                        }),
                        cost_usd: Some(0.125),
                        ..sdk::CaseResult::default()
                    }
                }
            }
        },
    )
    .await
    .unwrap()
}

fn input(report: &sdk::Report, baseline: Option<Baseline>, failing: Option<&str>) -> RunInput {
    RunInput {
        url: report.run.url.clone(),
        gate: Gate {
            pass_rate: Some(1.0),
            ..Gate::default()
        },
        ..run(TRIALS, cases(failing), baseline)
    }
}

#[tokio::test]
async fn summary_matches_dev_server() {
    let server = serve().await;
    let first = dev_run(&server, "main", "base", None).await;
    assert_eq!(
        ours(input(&first, None, None)).await,
        serde_json::to_value(first.summary().unwrap()).unwrap()
    );
    let prior = all_passing(&first.run.id, &first.run.version, &first.run.url);
    let broken = dev_run(&server, "topic", "broken", Some("case-0")).await;
    let expected = serde_json::to_value(broken.summary().unwrap()).unwrap();
    assert_eq!(
        ours(input(&broken, Some(prior), Some("case-0"))).await,
        expected
    );
    assert_eq!(expected["regressions"][0]["critical"], true);
    assert_eq!(expected["errors"], 3);
}
