use std::sync::Arc;

use chrono::DateTime;
use lens_server::tracing::{TraceAgent, TraceReadError, TraceReader};
use litellm_traces::{
    QueryScope, SpanDetail, SpanErrorPage, Trace, TraceConversationPage, TracePage,
    query::named::ReadAccessParams,
    request::{
        CONVERSATION_PAGE_SIZE_DEFAULT, TraceConversationRequest, TraceDetailRequest,
        TraceErrorPageRequest, TraceSpanRequest,
    },
};
use litellm_traces_cache::ReadError;
use litellm_traces_clickhouse::{
    ClickHouseTraces, Error, QueryHelp,
    query::lens::{TraceAgents, TraceAgentsParams},
};
use tokio::sync::OwnedSemaphorePermit;

use crate::{State, wait_for_read_slot};

pub struct TraceApi(pub Arc<State>);

impl TraceApi {
    async fn permit(&self) -> Result<OwnedSemaphorePermit, TraceReadError> {
        self.0
            .require_storage()
            .map_err(|_| TraceReadError::Unavailable)?;
        wait_for_read_slot(self.0.read_slots.clone().acquire_owned())
            .await
            .map_err(|_| TraceReadError::Unavailable)
    }

    fn traces(&self) -> ClickHouseTraces {
        ClickHouseTraces::new(
            self.0.storage.client.clone(),
            self.0.storage.config.storage().reader().clone(),
        )
    }
}

impl TraceReader for TraceApi {
    type Help = QueryHelp;

    async fn list(
        &self,
        scope: ReadAccessParams,
        start_ms: i64,
        end_ms: i64,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<TracePage, TraceReadError> {
        let _permit = self.permit().await?;
        self.0
            .storage
            .reader
            .list_traces(
                &self.traces(),
                &scope,
                start_ms,
                end_ms,
                cursor.as_deref(),
                limit,
            )
            .await
            .map_err(read_error)
    }

    async fn agents(
        &self,
        scope: ReadAccessParams,
        start_ms: i64,
        end_ms: i64,
        limit: u32,
    ) -> Result<Vec<TraceAgent>, TraceReadError> {
        let _permit = self.permit().await?;
        let rows = litellm_storage_clickhouse::fetch::<TraceAgents>(
            &self.0.storage.client,
            self.0.storage.config.storage().reader(),
            &TraceAgentsParams {
                all_teams: scope.all_teams,
                user_id: scope.user_id,
                team_ids: scope.team_ids,
                start_ms,
                end_ms,
                limit,
            },
        )
        .await
        .map_err(|_| TraceReadError::Unavailable)?;
        rows.into_iter()
            .map(|row| {
                let last_seen = i64::try_from(row.last_seen_ms)
                    .ok()
                    .and_then(DateTime::from_timestamp_millis)
                    .ok_or(TraceReadError::Unavailable)?;
                Ok(TraceAgent {
                    name: row.agent_name,
                    runs: row.runs,
                    failed_runs: row.failed_runs,
                    last_seen,
                    frameworks: row.frameworks,
                })
            })
            .collect()
    }

    async fn trace(
        &self,
        scope: ReadAccessParams,
        trace_id: String,
        request: TraceDetailRequest,
    ) -> Result<Option<Trace>, TraceReadError> {
        let _permit = self.permit().await?;
        let store = self.traces();
        match request.page_size {
            Some(size) => {
                self.0
                    .storage
                    .reader
                    .get_trace_page(
                        &store,
                        &scope,
                        &trace_id,
                        &request.trace_ref,
                        request.cursor.as_deref(),
                        u32::from(size),
                    )
                    .await
            }
            None if request.cursor.is_some() => {
                return Err(TraceReadError::InvalidRequest(
                    "cursor requires page_size".into(),
                ));
            }
            None => {
                self.0
                    .storage
                    .reader
                    .get_trace(&store, &scope, &trace_id, &request.trace_ref)
                    .await
            }
        }
        .map_err(read_error)
    }

    async fn span(
        &self,
        scope: ReadAccessParams,
        trace_id: String,
        span_id: String,
        request: TraceSpanRequest,
    ) -> Result<Option<SpanDetail>, TraceReadError> {
        let _permit = self.permit().await?;
        self.0
            .storage
            .reader
            .get_span(
                &self.traces(),
                &scope,
                &trace_id,
                &span_id,
                &request.trace_ref,
            )
            .await
            .map_err(read_error)
    }

    async fn conversation(
        &self,
        scope: ReadAccessParams,
        trace_id: String,
        request: TraceConversationRequest,
    ) -> Result<Option<TraceConversationPage>, TraceReadError> {
        let _permit = self.permit().await?;
        self.0
            .storage
            .reader
            .get_conversation(
                &self.traces(),
                &scope,
                &trace_id,
                &request.trace_ref,
                request.cursor.as_deref(),
                request.page_size.unwrap_or(CONVERSATION_PAGE_SIZE_DEFAULT),
            )
            .await
            .map_err(read_error)
    }

    async fn span_error(
        &self,
        scope: ReadAccessParams,
        trace_id: String,
        span_id: String,
        request: TraceErrorPageRequest,
    ) -> Result<Option<SpanErrorPage>, TraceReadError> {
        let _permit = self.permit().await?;
        self.0
            .storage
            .reader
            .get_span_error(
                &self.traces(),
                &scope,
                &trace_id,
                &span_id,
                &request.trace_ref,
                request.cursor.as_deref(),
            )
            .await
            .map_err(read_error)
    }

    async fn query(
        &self,
        scope: QueryScope,
        sql: String,
    ) -> Result<serde_json::Value, TraceReadError> {
        let _permit = self.permit().await?;
        let storage = &self.0.storage;
        let _query = storage.query_readers.acquire().map_err(store_error)?;
        let connection = storage
            .query_readers
            .connection(&storage.client, &scope, &storage.query_secret)
            .await
            .map_err(store_error)?;
        let response = litellm_traces_clickhouse::query_sql(&storage.client, &connection, &sql)
            .await
            .map_err(store_error)?;
        sql_response(&response)
    }

    async fn help(&self, scope: QueryScope) -> Result<Self::Help, TraceReadError> {
        let _permit = self.permit().await?;
        let storage = &self.0.storage;
        let _query = storage.query_readers.acquire().map_err(store_error)?;
        let connection = storage
            .query_readers
            .connection(&storage.client, &scope, &storage.query_secret)
            .await
            .map_err(store_error)?;
        litellm_traces_clickhouse::query_help(&storage.client, &connection)
            .await
            .map_err(store_error)
    }
}

fn sql_response(response: &str) -> Result<serde_json::Value, TraceReadError> {
    #[derive(serde::Deserialize)]
    struct Envelope {
        data: Vec<serde_json::Map<String, serde_json::Value>>,
    }
    let envelope: Envelope =
        serde_json::from_str(response).map_err(|_| TraceReadError::Unavailable)?;
    serde_json::to_value(litellm_traces::response::TraceSQLResponse {
        data: envelope.data,
    })
    .map_err(|_| TraceReadError::Unavailable)
}

fn store_error(error: Error) -> TraceReadError {
    match error {
        Error::InvalidQuery | Error::InvalidParameters | Error::InvalidScope => {
            TraceReadError::InvalidRequest(error.to_string())
        }
        _ => TraceReadError::Unavailable,
    }
}

fn read_error(error: ReadError<Error>) -> TraceReadError {
    match error {
        ReadError::InvalidParameters | ReadError::InvalidCursor(_) | ReadError::AmbiguousTrace => {
            TraceReadError::InvalidRequest(error.to_string())
        }
        ReadError::TraceChanged => TraceReadError::Changed,
        ReadError::TooLarge => TraceReadError::TooLarge,
        ReadError::Store(_) | ReadError::Encode(_) => TraceReadError::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::{TraceReadError, sql_response};
    use rstest::rstest;
    use serde_json::json;

    #[rstest]
    fn public_sql_response_contains_only_query_rows() {
        let data = json!([{ "tool": "search", "nested": [1, true, null, {"x": "y"}] }]);
        let envelope = json!({"meta": [{"name": "tool", "type": "String"}], "data": data, "rows": 1, "statistics": {"elapsed": 0.1, "rows_read": 1, "bytes_read": 12}});
        assert_eq!(
            sql_response(&envelope.to_string()).unwrap(),
            json!({"data": data})
        );
    }

    #[rstest]
    #[case::malformed("{")]
    #[case::missing_data("{}")]
    #[case::wrong_container(r#"{"data":{}}"#)]
    #[case::scalar_row(r#"{"data":["row"]}"#)]
    #[case::null_row(r#"{"data":[null]}"#)]
    fn malformed_query_rows_fail_closed(#[case] response: &str) {
        assert!(matches!(
            sql_response(response),
            Err(TraceReadError::Unavailable)
        ));
    }
}
