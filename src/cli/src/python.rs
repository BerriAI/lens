use std::{path::Path, sync::Arc};

use lens_evals_sdk::{
    Error as SdkError, client, doctor, github,
    model::EvalSpec,
    setup::{self as sdk_setup, Settings},
};
use pyo3::{exceptions::PyValueError, prelude::*};
use serde_json::json;

use crate::{
    Error,
    arguments::{self, Operation},
    local::{self, AgentResult, Outcome, RunMeta, Target, Task, run, view},
    setup::{self, InitOptions},
};

pub fn sdk_error(value: SdkError) -> PyErr {
    Python::attach(|py| {
        let module = py.import("lens.errors")?;
        let exception = match &value {
            SdkError::Configuration(_) | SdkError::Invalid { .. } | SdkError::Toml(_) => module
                .getattr("ConfigurationError")?
                .call1((value.to_string(),))?,
            SdkError::Api { status, code } => {
                module
                    .getattr("ApiFailure")?
                    .call1((*status, code, value.hint()))?
            }
            _ => module
                .getattr("InfrastructureError")?
                .call1((value.to_string(),))?,
        };
        Ok::<_, PyErr>(PyErr::from_value(exception))
    })
    .unwrap_or_else(|error| error)
}

fn error(value: Error) -> PyErr {
    if let Error::Sdk(value) = value {
        return sdk_error(value);
    }
    Python::attach(|py| {
        let class = match &value {
            Error::Configuration(_) | Error::Invalid { .. } => "ConfigurationError",
            _ => "InfrastructureError",
        };
        let exception = py
            .import("lens.errors")?
            .getattr(class)?
            .call1((value.to_string(),))?;
        Ok::<_, PyErr>(PyErr::from_value(exception))
    })
    .unwrap_or_else(|error| error)
}

fn decode<T: serde::de::DeserializeOwned>(input: &str) -> PyResult<T> {
    serde_json::from_str(input).map_err(|_| {
        error(Error::Configuration(
            "Invalid SDK configuration or result payload",
        ))
    })
}

fn encode<T: serde::Serialize>(value: &T) -> PyResult<String> {
    serde_json::to_string(value).map_err(|value| sdk_error(SdkError::Response(value)))
}

#[pyfunction]
fn parse_cli(arguments: Vec<String>) -> PyResult<String> {
    encode(&arguments::parse(&arguments))
}

#[pyfunction]
fn control(py: Python<'_>, command: &str) -> PyResult<String> {
    let command: Operation = decode(command)?;
    py.detach(|| {
        let root = std::env::current_dir().map_err(Error::Io).map_err(error)?;
        match command {
            Operation::Init {
                dataset,
                project,
                base_url,
                task,
                demo,
                action_ref,
            } => setup::initialize(
                &root,
                InitOptions {
                    dataset,
                    project,
                    base_url,
                    task,
                    demo,
                    action: action_ref,
                },
            )
            .map_err(error),
            Operation::DevServer {
                host,
                port,
                dataset_file,
            } => {
                pyo3_async_runtimes::tokio::get_runtime()
                    .block_on(lens_evals_sdk::devserver::serve(
                        &host,
                        port,
                        dataset_file.as_deref(),
                    ))
                    .map_err(sdk_error)?;
                Ok(String::new())
            }
            Operation::Report { file, via_app } => {
                pyo3_async_runtimes::tokio::get_runtime()
                    .block_on(github::publish_file_mode(&root, &file, via_app))
                    .map_err(sdk_error)?;
                Ok(String::new())
            }
            Operation::ReportStart { name, pr } => {
                pyo3_async_runtimes::tokio::get_runtime()
                    .block_on(github::publish_progress(&root, &name, pr, false))
                    .map_err(sdk_error)?;
                Ok(String::new())
            }
            Operation::ReportFailed { name, pr } => {
                pyo3_async_runtimes::tokio::get_runtime()
                    .block_on(github::publish_progress(&root, &name, pr, true))
                    .map_err(sdk_error)?;
                Ok(String::new())
            }
            _ => Err(PyValueError::new_err(
                "Evaluation commands require the Python task boundary",
            )),
        }
    })
}

#[pyfunction]
fn demo(py: Python<'_>, fast: bool) -> PyResult<()> {
    crate::demo::run(crate::demo::Options::detect(fast), &mut || {
        py.check_signals().map_err(std::io::Error::other)
    })
    .map_err(|failure| match failure.downcast::<PyErr>() {
        Ok(interrupt) => interrupt,
        Err(failure) => error(Error::Io(failure)),
    })
}

fn here() -> PyResult<std::path::PathBuf> {
    std::env::current_dir().map_err(Error::Io).map_err(error)
}

fn view_error(failure: std::io::Error) -> PyErr {
    error(Error::Io(failure))
}

fn saved_run(root: &Path, run_id: &str) -> PyResult<(RunMeta, Vec<crate::local::Attempt>)> {
    if !root
        .join(".lens/runs")
        .join(run_id)
        .join("meta.json")
        .is_file()
    {
        return Err(error(Error::Invalid {
            message: "No saved run with this id in .lens/runs",
            value: run_id.to_owned(),
        }));
    }
    run::load_run(root, run_id).map_err(error)
}

#[pyfunction]
fn run_meta(run_id: &str) -> PyResult<String> {
    encode(&saved_run(&here()?, run_id)?.0)
}

#[pyfunction]
fn latest_run() -> PyResult<Option<String>> {
    Ok(run::latest_run(&here()?))
}

#[pyfunction]
fn show(run_id: Option<String>) -> PyResult<()> {
    let root = here()?;
    let id = run_id.or_else(|| run::latest_run(&root)).ok_or_else(|| {
        error(Error::Configuration(
            "No saved runs in .lens/runs. Start one with lens run",
        ))
    })?;
    let (meta, attempts) = saved_run(&root, &id)?;
    view::print_report(
        crate::demo::Options::detect(true),
        &format!("lens show {id}"),
        &run::summarize(&meta, &attempts),
    )
    .map_err(view_error)
}

fn call_agent(
    locals: &pyo3_async_runtimes::TaskLocals,
    callback: &Py<PyAny>,
    task: &Task,
    target: &Target,
) -> PyResult<impl std::future::Future<Output = PyResult<Py<PyAny>>> + Send + use<>> {
    Python::attach(|py| {
        let awaitable = callback.call1(py, (encode(task)?, encode(target)?))?;
        pyo3_async_runtimes::into_future_with_locals(locals, awaitable.into_bound(py))
    })
}

fn boundary_failure(message: &str) -> AgentResult {
    AgentResult {
        outcome: Outcome::Error {
            kind: "TaskError".to_owned(),
            message: message.to_owned(),
        },
        output: String::new(),
        cost: None,
        judge_cost: None,
        tool_calls: Vec::new(),
        metrics: local::Metrics::default(),
        stop_reason: None,
        check_error: None,
        check_detail: None,
        duration_seconds: None,
        run_id: None,
    }
}

#[pyfunction]
#[pyo3(signature = (command, targets, concurrency, fast, callback, tasks=None, config=None, resume=None, task_data=None, trials=1, config_hash=None))]
#[allow(clippy::too_many_arguments)]
fn run_local<'py>(
    py: Python<'py>,
    command: String,
    targets: String,
    concurrency: usize,
    fast: bool,
    callback: Py<PyAny>,
    tasks: Option<String>,
    config: Option<String>,
    resume: Option<String>,
    task_data: Option<String>,
    trials: usize,
    config_hash: Option<String>,
) -> PyResult<Bound<'py, PyAny>> {
    let root = here()?;
    let targets: Vec<Target> = decode(&targets)?;
    let resuming = resume.is_some();
    let meta = match (resume, tasks, config) {
        (Some(id), _, _) => RunMeta {
            concurrency,
            ..saved_run(&root, &id)?.0
        },
        (None, tasks, Some(config)) => RunMeta {
            id: run::new_run_id(),
            task_count: 0,
            tasks_file: tasks.unwrap_or_default(),
            config_file: config,
            targets: targets.clone(),
            concurrency,
            trials,
            task_data: None,
            config_hash: config_hash.clone(),
        },
        _ => {
            return Err(error(Error::Configuration(
                "lens run needs --config and tasks in the config or a CSV file",
            )));
        }
    };
    let tasks: Vec<Task> = match task_data {
        Some(data) => decode(&data)?,
        None if !meta.tasks_file.is_empty() => {
            run::load_tasks(&root.join(&meta.tasks_file)).map_err(error)?
        }
        None => {
            return Err(error(Error::Configuration(
                "Config must define tasks when no CSV file is provided",
            )));
        }
    };
    run::validate_tasks(&tasks).map_err(error)?;
    if resuming {
        run::validate_resume(&meta, &tasks, &targets, config_hash.as_deref()).map_err(error)?;
    }
    let meta = RunMeta {
        task_count: tasks.len(),
        task_data: Some(tasks.clone()),
        ..meta
    };
    let locals = Arc::new(pyo3_async_runtimes::tokio::get_current_locals(py)?);
    let callback = Arc::new(callback);
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        let invoke = |task: Task, target: Target| {
            let locals = locals.clone();
            let callback = callback.clone();
            async move {
                let output = match call_agent(&locals, &callback, &task, &target) {
                    Ok(future) => future.await,
                    Err(failure) => Err(failure),
                };
                match output.and_then(|value| Python::attach(|py| value.extract::<String>(py))) {
                    Ok(json) => serde_json::from_str::<AgentResult>(&json).unwrap_or_else(|_| {
                        boundary_failure("Agent boundary returned an invalid result")
                    }),
                    Err(_) => boundary_failure("Python agent could not be awaited"),
                }
            }
        };
        let mut live = view::Live::start(crate::demo::Options::detect(fast), &command, &meta)
            .map_err(view_error)?;
        let mut failure: Option<std::io::Error> = None;
        let summary = run::execute(
            &root,
            meta,
            tasks,
            invoke,
            &mut |progress: local::Progress| {
                if failure.is_none() {
                    failure = live.update(&progress).err();
                }
            },
        )
        .await
        .map_err(error)?;
        if let Some(failure) = failure {
            return Err(view_error(failure));
        }
        live.finish(&summary).map_err(view_error)?;
        encode(&summary)
    })
}

#[pyfunction]
fn diagnose(py: Python<'_>, specs: String, ci: bool) -> PyResult<Bound<'_, PyAny>> {
    let specs: Vec<EvalSpec> = decode(&specs)?;
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        let root = std::env::current_dir()
            .map_err(SdkError::Io)
            .map_err(sdk_error)?;
        let configured = Settings::load(&root).map_err(sdk_error)?;
        let checks = match (|| {
            if configured.project.trim().is_empty() {
                return Err(SdkError::Configuration(
                    "Set [tool.lens].project to the agent.name on your traces",
                ));
            }
            let context = sdk_setup::execution(&root, ci)?;
            let client = client::Client::new(
                &configured.endpoint()?,
                &sdk_setup::env_value("LENS_API_KEY").unwrap_or_default(),
            )?;
            Ok((context, client))
        })() {
            Ok((context, client)) => {
                let checked = doctor::diagnose(&client, &configured, &specs).await;
                std::iter::once(doctor::Check::success(
                    "version",
                    format!("{} on {}", context.version, context.branch),
                ))
                .chain(checked)
                .collect::<Vec<_>>()
            }
            Err(value) => vec![doctor::Check::failure("configuration", value)],
        };
        encode(&json!({"ok":checks.iter().all(|check| check.ok),"checks":checks}))
    })
}

pub fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(parse_cli, module)?)?;
    module.add_function(wrap_pyfunction!(control, module)?)?;
    module.add_function(wrap_pyfunction!(demo, module)?)?;
    module.add_function(wrap_pyfunction!(diagnose, module)?)?;
    module.add_function(wrap_pyfunction!(run_local, module)?)?;
    module.add_function(wrap_pyfunction!(run_meta, module)?)?;
    module.add_function(wrap_pyfunction!(latest_run, module)?)?;
    module.add_function(wrap_pyfunction!(show, module)?)?;
    Ok(())
}
