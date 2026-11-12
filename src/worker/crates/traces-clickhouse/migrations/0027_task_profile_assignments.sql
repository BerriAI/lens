CREATE TABLE IF NOT EXISTS {database}.lens_task_profile_assignments
(
    TeamId LowCardinality(String),
    ApiKeyHash String,
    TraceId String,
    UserIds Array(String),
    AgentName String,
    TaxonomyId String,
    TaxonomyVersion UInt32,
    TaskId String,
    Outcome LowCardinality(String),
    StartTs DateTime64(3),
    ComputedAt UInt64
)
ENGINE = ReplacingMergeTree(ComputedAt)
ORDER BY (TeamId, ApiKeyHash, TraceId, TaxonomyId, TaxonomyVersion)
SETTINGS materialize_ttl_recalculate_only = 1, non_replicated_deduplication_window = 1000
