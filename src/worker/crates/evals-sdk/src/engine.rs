use std::{
    future::Future,
    time::{Duration, Instant},
};

use futures_util::{StreamExt, TryStreamExt, stream};
use sha2::{Digest, Sha256};

use crate::{Error, Result, client::Client, model::*};

pub fn redact(message: &str) -> String {
    std::env::vars()
        .filter(|(name, value)| {
            value.len() >= 8
                && ["KEY", "TOKEN", "SECRET", "PASSWORD"]
                    .iter()
                    .any(|word| name.to_uppercase().contains(word))
        })
        .fold(message.to_owned(), |text, (_, secret)| {
            text.replace(&secret, "[redacted]")
        })
}

pub async fn evaluate<F, Fut>(
    client: &Client,
    spec: &EvalSpec,
    project: &str,
    execution: &Execution,
    task: F,
) -> Result<Report>
where
    F: Fn(Case) -> Fut + Send + Sync,
    Fut: Future<Output = CaseResult> + Send,
{
    spec.validate()?;
    evaluate_managed(client, spec, project, execution, |case| {
        let future = task(case);
        async move {
            match tokio::time::timeout(Duration::from_secs_f64(spec.timeout_seconds), future).await
            {
                Ok(result) => result,
                Err(_) => failure("TimeoutError", "Task exceeded timeout_per_trial"),
            }
        }
    })
    .await
}

/// The callback must enforce its deadline and finish cancellation before resolving.
/// Python coroutine cleanup cannot be completed by dropping its Rust future.
pub async fn evaluate_managed<F, Fut>(
    client: &Client,
    spec: &EvalSpec,
    project: &str,
    execution: &Execution,
    task: F,
) -> Result<Report>
where
    F: Fn(Case) -> Fut + Send + Sync,
    Fut: Future<Output = CaseResult> + Send,
{
    spec.validate()?;
    if project.trim().is_empty() {
        return Err(Error::Configuration(
            "Set [tool.lens].project to the agent.name on your traces",
        ));
    }
    let dataset = client.resolve(&spec.data).await?;
    let cases = spec.select(&client.cases(&dataset).await?)?;
    let body = CreateEvalRun {
        eval: spec.name.clone(),
        agent: project.to_owned(),
        dataset_id: dataset.id,
        revision: dataset.revision,
        case_ids: Some(cases.iter().map(|case| case.id.clone()).collect()),
        baseline_run_id: execution.baseline_run_id.clone(),
        version: execution.version.clone(),
        branch: execution.branch.clone(),
        pr: execution.pr,
        ci_url: execution.ci_url.clone(),
        trials: spec.trials,
        timeout_per_trial_ms: spec.timeout_millis()?,
        scorers: spec.scores.clone(),
        gate: spec.gate.clone(),
        agent_io: None,
    };
    evaluate_resolved(client, spec, execution, body, cases, |case, _| task(case)).await
}

pub(crate) async fn evaluate_resolved<F, Fut>(
    client: &Client,
    spec: &EvalSpec,
    execution: &Execution,
    body: CreateEvalRun,
    cases: Vec<Case>,
    task: F,
) -> Result<Report>
where
    F: Fn(Case, String) -> Fut + Send + Sync,
    Fut: Future<Output = CaseResult> + Send,
{
    let run = create_run(client, spec, execution, &body).await?;
    let trials = if matches!(run.status, RunStatus::Running) {
        let work = cases
            .iter()
            .flat_map(|case| (0..spec.trials).map(move |trial| (case.clone(), trial)))
            .collect::<Vec<_>>();
        stream::iter(work)
            .map(|(case, trial)| trial_result(client, &run.id, case, trial, &task))
            .buffer_unordered(spec.concurrency)
            .try_collect::<Vec<_>>()
            .await?
    } else {
        Vec::new()
    };
    complete_run(client, spec, run, trials).await
}

pub(crate) async fn create_run(
    client: &Client,
    spec: &EvalSpec,
    execution: &Execution,
    body: &CreateEvalRun,
) -> Result<EvalRun> {
    let fingerprint = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&body).map_err(Error::Response)?)
    );
    let key = format!(
        "{}:{}:{}:{}",
        spec.name, execution.version, execution.identity, fingerprint
    );
    let run = client.create(body, &key).await?;
    if run.eval != spec.name || run.version != execution.version || run.agent != body.agent {
        return Err(Error::Infrastructure(
            "Lens created a run for a different evaluation",
        ));
    }
    Ok(run)
}

pub(crate) async fn complete_run(
    client: &Client,
    spec: &EvalSpec,
    run: EvalRun,
    trials: Vec<TrialResult>,
) -> Result<Report> {
    let finished = if matches!(run.status, RunStatus::Running) {
        client.finish(&run.id).await?
    } else {
        run
    };
    let completed = wait(
        client,
        finished,
        Duration::from_secs_f64(spec.timeout_seconds + 60.0),
    )
    .await?;
    let baseline = match completed
        .summary
        .as_ref()
        .and_then(|summary| summary.baseline_run_id.as_deref())
    {
        Some(id) => Some(client.get(id, false).await?),
        None => None,
    };
    let report = Report {
        run: completed,
        baseline,
        trials,
    };
    report.summary()?;
    Ok(report)
}

async fn trial_result<F, Fut>(
    client: &Client,
    run_id: &str,
    case: Case,
    trial: usize,
    task: &F,
) -> Result<TrialResult>
where
    F: Fn(Case, String) -> Fut + Send + Sync,
    Fut: Future<Output = CaseResult> + Send,
{
    let started = Instant::now();
    let request_id = format!(
        "lens-{:x}",
        Sha256::digest(serde_json::to_vec(&(run_id, &case.id, trial)).map_err(Error::Response)?)
    );
    let result = task(case.clone(), request_id).await;
    let result = if result.validate().is_ok() {
        result
    } else {
        failure("ConfigurationError", "Task must return a valid lens.Run")
    };
    let result = CaseResult {
        error: result.error.map(|error| CaseError {
            message: redact(&error.message),
            ..error
        }),
        duration_ms: Some(started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)),
        ..result
    };
    client.result(run_id, &case.id, trial, &result).await?;
    Ok(TrialResult {
        case_id: case.id,
        trial,
        result,
    })
}

pub fn failure(kind: &str, message: &str) -> CaseResult {
    CaseResult {
        error: Some(CaseError {
            r#type: kind.to_owned(),
            message: redact(message),
        }),
        ..CaseResult::default()
    }
}

async fn wait(client: &Client, run: EvalRun, timeout: Duration) -> Result<EvalRun> {
    tokio::time::timeout(timeout, async {
        let mut current = run;
        loop {
            match current.status {
                RunStatus::Done => {
                    if current.summary.is_none() {
                        return Err(Error::Infrastructure(
                            "Lens completed the run without a summary",
                        ));
                    }
                    return Ok(current);
                }
                RunStatus::Failed => {
                    return Err(Error::Infrastructure(
                        "Lens failed to score the run; see its run page",
                    ));
                }
                RunStatus::Running | RunStatus::Scoring => {}
            }
            current = client.get(&current.id, true).await?;
            if matches!(current.status, RunStatus::Running | RunStatus::Scoring) {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    })
    .await
    .map_err(|_| Error::Infrastructure("Timed out waiting for Lens to finish scoring"))?
}
