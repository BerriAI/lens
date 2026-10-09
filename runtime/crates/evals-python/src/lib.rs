use std::{path::Path, sync::Arc};

use lens_evals_sdk::{
    Error, client, doctor, engine, github,
    model::{Case, CaseResult, EvalSpec, Execution, Report},
    setup::{self, InitOptions, Operation, Settings},
};
use pyo3::{exceptions::PyValueError, prelude::*};
use serde_json::json;

fn error(value: Error) -> PyErr {
    Python::attach(|py| {
        let module = py.import("lens.errors")?;
        let exception = match &value {
            Error::Configuration(_) | Error::Invalid { .. } | Error::Toml(_) => module
                .getattr("ConfigurationError")?
                .call1((value.to_string(),))?,
            Error::Api { status, code } => {
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

fn decode<T: serde::de::DeserializeOwned>(input: &str) -> PyResult<T> {
    serde_json::from_str(input).map_err(|_| {
        error(Error::Configuration(
            "Invalid SDK configuration or result payload",
        ))
    })
}
fn encode<T: serde::Serialize>(value: &T) -> PyResult<String> {
    serde_json::to_string(value).map_err(|value| error(Error::Response(value)))
}

#[pyfunction]
fn validate_eval(spec: &str) -> PyResult<()> {
    decode::<EvalSpec>(spec)?.validate().map_err(error)
}

#[pyfunction]
fn subset_name(
    name: &str,
    finding: Option<String>,
    case_ids: Option<Vec<String>>,
) -> PyResult<String> {
    lens_evals_sdk::model::subset_name(name, finding, case_ids).map_err(error)
}

#[pyfunction]
fn settings(root: &str) -> PyResult<String> {
    encode(&Settings::load(Path::new(root)).map_err(error)?)
}

#[pyfunction]
fn execution(root: &str, ci: bool) -> PyResult<String> {
    encode(&setup::execution(Path::new(root), ci).map_err(error)?)
}

#[pyfunction]
fn endpoint(value: &str) -> PyResult<String> {
    client::endpoint(value).map_err(error)
}

#[pyfunction]
fn redact(value: &str) -> String {
    engine::redact(value)
}

#[pyfunction]
fn parse_cli(arguments: Vec<String>) -> PyResult<String> {
    encode(&setup::parse(&arguments))
}

#[pyfunction]
fn render(report: &str, format: &str) -> PyResult<String> {
    let report: Report = decode(report)?;
    match format {
        "markdown" => lens_evals_sdk::reporting::markdown(&report).map_err(error),
        "terminal" => lens_evals_sdk::reporting::terminal(&report).map_err(error),
        "conclusion" => lens_evals_sdk::reporting::conclusion(&report)
            .map(str::to_owned)
            .map_err(error),
        _ => Err(PyValueError::new_err("Unknown report format")),
    }
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
                    .map_err(error)?;
                Ok(String::new())
            }
            Operation::Report { file } => {
                pyo3_async_runtimes::tokio::get_runtime()
                    .block_on(github::publish_file(&root, &file))
                    .map_err(error)?;
                Ok(String::new())
            }
            _ => Err(PyValueError::new_err(
                "Evaluation commands require the Python task boundary",
            )),
        }
    })
}

#[pyfunction]
fn diagnose(py: Python<'_>, specs: String, ci: bool) -> PyResult<Bound<'_, PyAny>> {
    let specs: Vec<EvalSpec> = decode(&specs)?;
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        let root = std::env::current_dir().map_err(Error::Io).map_err(error)?;
        let configured = Settings::load(&root).map_err(error)?;
        let checks = match (|| {
            if configured.project.trim().is_empty() {
                return Err(Error::Configuration(
                    "Set [tool.lens].project to the agent.name on your traces",
                ));
            }
            let context = setup::execution(&root, ci)?;
            let client = client::Client::new(
                &configured.endpoint()?,
                &setup::env_value("LENS_API_KEY").unwrap_or_default(),
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

#[pyfunction]
fn evaluate<'py>(
    py: Python<'py>,
    spec: String,
    project: String,
    endpoint: String,
    key: String,
    context: String,
    callback: Py<PyAny>,
) -> PyResult<Bound<'py, PyAny>> {
    let spec: EvalSpec = decode(&spec)?;
    let context: Execution = decode(&context)?;
    let client = client::Client::new(&endpoint, &key).map_err(error)?;
    let locals = Arc::new(pyo3_async_runtimes::tokio::get_current_locals(py)?);
    let callback = Arc::new(callback);
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        let task = |case: Case| {
            let locals = locals.clone();
            let callback = callback.clone();
            async move {
                let future = Python::attach(|py| {
                    let awaitable = callback.call1(py, (encode(&case)?,))?;
                    pyo3_async_runtimes::into_future_with_locals(&locals, awaitable.into_bound(py))
                });
                let output = match future {
                    Ok(future) => future.await,
                    Err(error) => Err(error),
                };
                match output.and_then(|value| Python::attach(|py| value.extract::<String>(py))) {
                    Ok(output) => {
                        serde_json::from_str::<CaseResult>(&output).unwrap_or_else(|_| {
                            engine::failure(
                                "ConfigurationError",
                                "Task boundary returned an invalid result",
                            )
                        })
                    }
                    Err(_) => engine::failure("TaskError", "Python task could not be awaited"),
                }
            }
        };
        encode(
            &engine::evaluate(&client, &spec, &project, &context, task)
                .await
                .map_err(error)?,
        )
    })
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(validate_eval, module)?)?;
    module.add_function(wrap_pyfunction!(settings, module)?)?;
    module.add_function(wrap_pyfunction!(subset_name, module)?)?;
    module.add_function(wrap_pyfunction!(execution, module)?)?;
    module.add_function(wrap_pyfunction!(endpoint, module)?)?;
    module.add_function(wrap_pyfunction!(redact, module)?)?;
    module.add_function(wrap_pyfunction!(parse_cli, module)?)?;
    module.add_function(wrap_pyfunction!(render, module)?)?;
    module.add_function(wrap_pyfunction!(control, module)?)?;
    module.add_function(wrap_pyfunction!(diagnose, module)?)?;
    module.add_function(wrap_pyfunction!(evaluate, module)?)?;
    Ok(())
}
