#![allow(
    dead_code,
    reason = "Each integration test binary uses a different subset of these fixtures"
)]

use lens_evals_sdk::{client::Client, devserver, model::*};
use rstest::fixture;

pub struct Server {
    pub client: Client,
    pub address: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub async fn serve(app: axum::Router) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Server {
        client: Client::new(&address, "lens-dev").unwrap(),
        address,
        task,
    }
}

#[fixture]
pub async fn server() -> Server {
    serve(devserver::router(
        devserver::sample_cases(36),
        "lens-dev".into(),
    ))
    .await
}

#[fixture]
pub fn spec() -> EvalSpec {
    EvalSpec {
        name: "demo".into(),
        data: "demo@1".into(),
        scores: vec![Scorer::TaskCompleted],
        baseline: "main".into(),
        trials: 3,
        gate: Gate {
            pass_rate: Some(1.0),
            ..Gate::default()
        },
        concurrency: 4,
        timeout_seconds: 3.0,
        finding: None,
        case_ids: None,
    }
}

pub fn context(branch: &str, identity: &str) -> Execution {
    Execution {
        version: "sha".into(),
        branch: branch.into(),
        pr: (branch != "main").then_some(7),
        ci_url: String::new(),
        identity: identity.into(),
    }
}
pub fn good() -> CaseResult {
    CaseResult {
        trace: Some(TraceRef {
            attribute: "session.id".into(),
            value: "pass".into(),
        }),
        cost_usd: Some(0.01),
        ..CaseResult::default()
    }
}
