use std::sync::Arc;

use lens_datasets::{DatasetReader, Finding, ReadError, Scope};
use litellm_storage_clickhouse::datasets::Findings;
use litellm_traces::{SpanDetail, Trace, query::named::ReadAccessParams};
use litellm_traces_cache::{ReadError as TraceReadError, TraceReader};

use crate::{ClickHouseTraces, Error};

pub struct Reader {
    traces: ClickHouseTraces,
    reader: Arc<TraceReader>,
    findings: Findings,
}

impl Reader {
    pub fn new(traces: ClickHouseTraces, reader: Arc<TraceReader>, findings: Findings) -> Self {
        Self {
            traces,
            reader,
            findings,
        }
    }
}

const ALL_TRACES: ReadAccessParams = ReadAccessParams {
    all_teams: true,
    user_id: String::new(),
    team_ids: Vec::new(),
};
const PAGE_SIZE: u32 = 500;

impl DatasetReader for Reader {
    async fn trace(&self, trace_id: &str, trace_ref: &str) -> Result<Option<Trace>, ReadError> {
        let Some(mut trace) = self
            .reader
            .get_trace_page(
                &self.traces,
                &ALL_TRACES,
                trace_id,
                trace_ref,
                None,
                PAGE_SIZE,
            )
            .await
            .map_err(failure)?
        else {
            return Ok(None);
        };
        while let Some(cursor) = trace.next_cursor.take() {
            let Some(page) = self
                .reader
                .get_trace_page(
                    &self.traces,
                    &ALL_TRACES,
                    trace_id,
                    trace_ref,
                    Some(&cursor),
                    PAGE_SIZE,
                )
                .await
                .map_err(failure)?
            else {
                return Ok(None);
            };
            trace.spans.extend(page.spans);
            trace.next_cursor = page.next_cursor;
        }
        Ok(Some(trace))
    }

    async fn span(
        &self,
        trace_id: &str,
        span_id: &str,
        trace_ref: &str,
    ) -> Result<Option<SpanDetail>, ReadError> {
        self.reader
            .get_span(&self.traces, &ALL_TRACES, trace_id, span_id, trace_ref)
            .await
            .map_err(failure)
    }

    async fn findings(
        &self,
        lens_id: &str,
        ids: &[String],
        scope: &Scope,
    ) -> Result<Vec<Finding>, ReadError> {
        self.findings.get(lens_id, ids, scope).await
    }
}

fn failure(error: TraceReadError<Error>) -> ReadError {
    match error {
        TraceReadError::InvalidParameters
        | TraceReadError::InvalidCursor(_)
        | TraceReadError::AmbiguousTrace => ReadError::InvalidRequest(Box::new(error)),
        TraceReadError::TraceChanged => ReadError::TraceChanged(Box::new(error)),
        TraceReadError::TooLarge => ReadError::TooLarge,
        TraceReadError::Encode(_) | TraceReadError::Store(_) => {
            ReadError::Unavailable(Box::new(error))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::failure;
    use crate::Error;
    use lens_datasets::ReadError;
    use litellm_traces_cache::ReadError as TraceReadError;
    use rstest::rstest;
    use std::sync::Arc;

    #[rstest]
    #[case::parameters(TraceReadError::InvalidParameters, "invalid trace read parameters")]
    #[case::cursor(TraceReadError::InvalidCursor("span"), "Invalid span cursor")]
    #[case::ambiguous(
        TraceReadError::AmbiguousTrace,
        "Multiple traces have this ID; provide trace_ref"
    )]
    fn invalid_reads_preserve_diagnostic(
        #[case] error: TraceReadError<Error>,
        #[case] expected: &str,
    ) {
        let error = failure(error);
        assert!(matches!(&error, ReadError::InvalidRequest(_)));
        assert_eq!(error.to_string(), expected);
    }

    #[rstest]
    fn changed_trace_requires_a_new_traversal() {
        let error = failure(TraceReadError::TraceChanged);
        assert!(matches!(&error, ReadError::TraceChanged(_)));
        assert_eq!(
            error.to_string(),
            "Trace changed while paging; refresh the trace to continue"
        );
    }

    #[rstest]
    fn exceeded_budget_retains_the_size_failure() {
        assert!(matches!(
            failure(TraceReadError::TooLarge),
            ReadError::TooLarge
        ));
    }

    #[rstest]
    #[case::storage(TraceReadError::Store(Arc::new(Error::InvalidRow)))]
    #[case::encoding(TraceReadError::Encode(Arc::new(serde_json::from_str::<serde_json::Value>("{").unwrap_err())))]
    fn internal_failures_do_not_expose_storage_details(#[case] error: TraceReadError<Error>) {
        let error = failure(error);
        assert!(matches!(&error, ReadError::Unavailable(_)));
        assert_eq!(
            error.to_string(),
            "Traces are temporarily unavailable. Please try again."
        );
    }
}
