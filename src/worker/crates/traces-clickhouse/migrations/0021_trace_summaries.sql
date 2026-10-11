CREATE TABLE IF NOT EXISTS {database}.lens_trace_summaries
(
    TeamId LowCardinality(String),
    ApiKeyHash String,
    TraceId String,
    TraceRef String,
    ScopeUserId String,
    SourceSpanCount UInt64,
    StartTs DateTime64(3),
    ComputedAt UInt64,
    ResolutionVersion UInt32,
    SpendKeys Array(Tuple(String, String)),
    Summary String
)
ENGINE = ReplacingMergeTree(ComputedAt)
ORDER BY (TraceRef, ScopeUserId)
SETTINGS materialize_ttl_recalculate_only = 1, non_replicated_deduplication_window = 1000
