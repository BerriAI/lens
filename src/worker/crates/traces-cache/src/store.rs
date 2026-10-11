use std::future::Future;

use litellm_traces::query::named::{
    ListTracesParams, ListTracesRow, SpanDetailParams, SpanDetailRow, SpanErrorParams,
    SpanErrorRow, SpendByResponseIdsParams, SpendByResponseIdsRow, TraceConversationAnchor,
    TraceConversationAnchorParams, TraceConversationRow, TraceConversationTurnsParams,
    TraceIdentityParams, TracePageSpansParams, TraceSpansParams, TraceSpansRow,
};

#[derive(Debug, thiserror::Error)]
pub enum StoreError<E> {
    #[error("trace read exceeds the storage read budget")]
    TooLarge,
    #[error(transparent)]
    Failed(E),
}

#[derive(Clone)]
pub struct SummaryPage {
    pub data: Vec<litellm_traces::TraceSummary>,
    pub next_position: Option<(i64, String)>,
}

pub trait TraceStore: Sync {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Identifies the backing storage for snapshot cache keys.
    fn source(&self) -> &str;

    fn summary_page(
        &self,
        _params: &ListTracesParams,
    ) -> impl Future<Output = Result<Option<SummaryPage>, StoreError<Self::Error>>> + Send {
        async { Ok(None) }
    }

    fn summaries(
        &self,
        _access: &litellm_traces::query::named::ReadAccessParams,
        _runs: &[ListTracesRow],
    ) -> impl Future<Output = Result<Vec<litellm_traces::TraceSummary>, StoreError<Self::Error>>> + Send
    {
        async { Ok(Vec::new()) }
    }

    fn trace_refs(
        &self,
        params: &TraceIdentityParams,
    ) -> impl Future<Output = Result<Vec<String>, StoreError<Self::Error>>> + Send;

    fn conversation_anchor(
        &self,
        params: &TraceConversationAnchorParams,
    ) -> impl Future<Output = Result<Option<TraceConversationAnchor>, StoreError<Self::Error>>> + Send;

    fn conversation_turns(
        &self,
        params: &TraceConversationTurnsParams,
    ) -> impl Future<Output = Result<Vec<TraceConversationRow>, StoreError<Self::Error>>> + Send;

    /// Returns `TooLarge` when the response exceeds the storage limit so the reader can halve `limit`.
    fn list_runs(
        &self,
        params: &ListTracesParams,
    ) -> impl Future<Output = Result<Vec<ListTracesRow>, StoreError<Self::Error>>> + Send;

    /// Returns spans visible at `snapshot_ms`, sorted by `start_ns`.
    fn trace_spans(
        &self,
        params: &TraceSpansParams,
        snapshot_ms: u64,
    ) -> impl Future<Output = Result<Vec<TraceSpansRow>, StoreError<Self::Error>>> + Send;

    /// Returns spans visible at `snapshot_ms`, sorted by `start_ns`.
    fn run_spans(
        &self,
        params: &TracePageSpansParams,
        snapshot_ms: u64,
    ) -> impl Future<Output = Result<Vec<TraceSpansRow>, StoreError<Self::Error>>> + Send;

    fn spend(
        &self,
        params: &SpendByResponseIdsParams,
    ) -> impl Future<Output = Result<Vec<SpendByResponseIdsRow>, StoreError<Self::Error>>> + Send;

    fn span_detail(
        &self,
        params: &SpanDetailParams,
    ) -> impl Future<Output = Result<Option<SpanDetailRow>, StoreError<Self::Error>>> + Send;

    fn span_error(
        &self,
        params: &SpanErrorParams,
    ) -> impl Future<Output = Result<Option<SpanErrorRow>, StoreError<Self::Error>>> + Send;
}
