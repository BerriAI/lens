use std::{path::Path, sync::Arc};

use lens_cli::python::sdk_error as error;
use lens_evals_sdk::{
    Error, client, engine,
    model::{Case, CaseResult, EvalSpec, Execution, Report},
    setup::{self, Settings},
};
use pyo3::{exceptions::PyValueError, prelude::*};

mod session;

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
            &engine::evaluate_managed(&client, &spec, &project, &context, task)
                .await
                .map_err(error)?,
        )
    })
}

#[pyfunction]
fn run_named(
    py: Python<'_>,
    name: String,
    endpoint: String,
    key: String,
    context: String,
    root: String,
) -> PyResult<Bound<'_, PyAny>> {
    let context: Execution = decode(&context)?;
    pyo3_async_runtimes::tokio::future_into_py(py, async move {
        encode(
            &lens_evals_sdk::named::evaluate(&name, &endpoint, &key, &context, Path::new(&root))
                .await
                .map_err(error)?,
        )
    })
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    lens_cli::python::register(module)?;
    module.add_class::<session::SavedEvaluation>()?;
    module.add_function(wrap_pyfunction!(validate_eval, module)?)?;
    module.add_function(wrap_pyfunction!(settings, module)?)?;
    module.add_function(wrap_pyfunction!(subset_name, module)?)?;
    module.add_function(wrap_pyfunction!(execution, module)?)?;
    module.add_function(wrap_pyfunction!(endpoint, module)?)?;
    module.add_function(wrap_pyfunction!(redact, module)?)?;
    module.add_function(wrap_pyfunction!(render, module)?)?;
    module.add_function(wrap_pyfunction!(evaluate, module)?)?;
    module.add_function(wrap_pyfunction!(run_named, module)?)?;
    Ok(())
}
