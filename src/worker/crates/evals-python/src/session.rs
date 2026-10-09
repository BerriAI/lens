use lens_evals_sdk::{
    model::{CaseError, CaseResult, Execution},
    session::{Evaluation, TestCase},
};
use pyo3::prelude::*;

use crate::{decode, encode, error};

#[pyclass(name = "SavedEvaluation")]
pub(crate) struct SavedEvaluation {
    inner: Evaluation,
}

#[pymethods]
impl SavedEvaluation {
    #[new]
    fn new(py: Python<'_>, name: &str, endpoint: &str, key: &str, context: &str) -> PyResult<Self> {
        let context: Execution = decode(context)?;
        py.detach(|| {
            let inner = pyo3_async_runtimes::tokio::get_runtime()
                .block_on(Evaluation::start(name, endpoint, key, &context))
                .map_err(error)?;
            Ok(Self { inner })
        })
    }

    fn cases(&self) -> PyResult<String> {
        encode(&self.inner.cases())
    }

    fn run(&self) -> PyResult<String> {
        encode(self.inner.run())
    }

    fn record(&mut self, py: Python<'_>, case: &str, result: &str) -> PyResult<()> {
        let case: TestCase = decode(case)?;
        let result: CaseResult = decode(result)?;
        py.detach(|| {
            pyo3_async_runtimes::tokio::get_runtime()
                .block_on(self.inner.record(&case, result))
                .map_err(error)
        })
    }

    fn finish(&mut self, py: Python<'_>, failure: &str) -> PyResult<String> {
        let failure: Option<CaseError> = decode(failure)?;
        py.detach(|| {
            encode(
                &pyo3_async_runtimes::tokio::get_runtime()
                    .block_on(self.inner.finish(failure))
                    .map_err(error)?,
            )
        })
    }
}
