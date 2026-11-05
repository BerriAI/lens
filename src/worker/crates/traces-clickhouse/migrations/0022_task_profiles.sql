CREATE TABLE IF NOT EXISTS {database}.lens_task_profiles
(
    TeamId LowCardinality(String),
    ApiKeyHash String,
    TraceId String,
    UserIds Array(String),
    AgentName String,
    ServiceName String,
    ClusterId String,
    Category String,
    StartTs DateTime64(3),
    ComputedAt UInt64,
    ResolutionVersion UInt32
)
ENGINE = ReplacingMergeTree(ComputedAt)
ORDER BY (TeamId, ApiKeyHash, TraceId)
SETTINGS materialize_ttl_recalculate_only = 1, non_replicated_deduplication_window = 1000
