CREATE TABLE IF NOT EXISTS {database}.lens_task_clusters
(
    TeamId LowCardinality(String),
    ApiKeyHash String,
    OwnershipHash String,
    AgentName String,
    ServiceName String,
    ClusterId String,
    Category String,
    Embedding Array(Float32),
    UpdatedAt DateTime64(3),
    ComputedAt UInt64,
    ResolutionVersion UInt32
)
ENGINE = ReplacingMergeTree(ComputedAt)
ORDER BY (TeamId, ApiKeyHash, OwnershipHash, AgentName, ServiceName, ClusterId)
SETTINGS materialize_ttl_recalculate_only = 1, non_replicated_deduplication_window = 1000
