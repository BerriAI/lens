CREATE TABLE IF NOT EXISTS {database}.lens_trace_tags
(
    TeamId LowCardinality(String),
    ApiKeyHash String,
    TraceId String CODEC(ZSTD(1)),
    Tags Map(String, String),
    UpdatedAt DateTime64(6),
    EngineReceivedMs UInt64 DEFAULT 0
)
ENGINE = ReplacingMergeTree(UpdatedAt)
ORDER BY (TeamId, ApiKeyHash, TraceId)
SETTINGS materialize_ttl_recalculate_only = 1, non_replicated_deduplication_window = 1000
