use std::{path::Path, time::Duration};

use crate::{
    Error, Result,
    agent::Executor,
    client::{Client, value},
    engine,
    model::{Case, CreateEvalRun, EvalSpec, Execution, Report},
    setup::{self, Settings},
};

pub(crate) struct PreparedEvaluation {
    pub client: Client,
    pub spec: EvalSpec,
    pub execution: Execution,
    pub body: CreateEvalRun,
    pub cases: Vec<Case>,
}

pub(crate) async fn prepare(
    name: &str,
    endpoint: &str,
    key: &str,
    execution: &Execution,
) -> Result<PreparedEvaluation> {
    if execution.version.trim().is_empty() || execution.branch.trim().is_empty() {
        return Err(Error::Configuration(
            "An eval execution requires a version and branch",
        ));
    }
    let execution = Execution {
        identity: if execution.identity.trim().is_empty() {
            uuid::Uuid::new_v4().simple().to_string()
        } else {
            execution.identity.clone()
        },
        ..execution.clone()
    };
    let client = Client::named(endpoint, key)?;
    let saved = client.definition(name).await?.spec;
    let spec = EvalSpec {
        name: name.to_owned(),
        data: saved.dataset_id.clone(),
        scores: serde_json::from_value(value(&saved.scorers)?).map_err(Error::Response)?,
        baseline: saved.baseline,
        trials: saved.trials as usize,
        gate: serde_json::from_value(value(&saved.gate)?).map_err(Error::Response)?,
        concurrency: 8,
        timeout_seconds: saved.timeout_per_trial_ms as f64 / 1000.0,
        finding: None,
        case_ids: None,
    };
    spec.validate()?;
    if saved.agent.trim().is_empty() {
        return Err(Error::Configuration(
            "The saved eval requires an agent name",
        ));
    }
    let dataset = client.dataset(&saved.dataset_id, saved.revision).await?;
    let cases = spec.select(&client.cases(&dataset).await?)?;
    let body = CreateEvalRun {
        eval: name.to_owned(),
        agent: saved.agent,
        dataset_id: dataset.id,
        revision: dataset.revision,
        case_ids: Some(cases.iter().map(|case| case.id.clone()).collect()),
        baseline_run_id: execution.baseline_run_id.clone(),
        version: execution.version.clone(),
        branch: execution.branch.clone(),
        pr: execution.pr,
        ci_url: execution.ci_url.clone(),
        trials: spec.trials,
        timeout_per_trial_ms: saved.timeout_per_trial_ms,
        scorers: spec.scores.clone(),
        gate: spec.gate.clone(),
        agent_io: saved.agent_io,
    };
    Ok(PreparedEvaluation {
        client,
        spec,
        execution,
        body,
        cases,
    })
}

pub async fn evaluate(
    name: &str,
    endpoint: &str,
    key: &str,
    execution: &Execution,
    root: &Path,
) -> Result<Report> {
    evaluate_with_environment(name, endpoint, key, execution, root, setup::env_value).await
}

pub async fn evaluate_with_environment(
    name: &str,
    endpoint: &str,
    key: &str,
    execution: &Execution,
    root: &Path,
    environment: impl Fn(&str) -> Option<String>,
) -> Result<Report> {
    let prepared = prepare(name, endpoint, key, execution).await?;
    let io = prepared.body.agent_io.clone().ok_or(Error::Configuration(
        "The saved eval requires an agent_io contract",
    ))?;
    io.validate()
        .map_err(|_| Error::Configuration("Invalid saved agent_io contract"))?;
    let configured = Settings::load(root)?;
    let connection = configured
        .connections
        .get(&io.connection)
        .ok_or(Error::Configuration(
            "Configure the saved agent connection in [tool.lens.connections]",
        ))?
        .resolve(environment)?;
    let executor = Executor::connect(io, connection).await?;
    let timeout = Duration::from_millis(prepared.body.timeout_per_trial_ms);
    engine::evaluate_resolved(
        &prepared.client,
        &prepared.spec,
        &prepared.execution,
        prepared.body,
        prepared.cases,
        |case, request_id| executor.run(case, request_id, timeout),
    )
    .await
}
