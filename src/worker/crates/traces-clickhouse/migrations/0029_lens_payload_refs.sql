CREATE TABLE IF NOT EXISTS {database}.lens_payload_refs
(
    team_id LowCardinality(String),
    api_key String,
    event_id String,
    field LowCardinality(String),
    event_time DateTime64(3),
    object_key String,
    bytes UInt64,
    sha256 String,
    EngineReceivedMs UInt64 DEFAULT 0
)
ENGINE = ReplacingMergeTree(EngineReceivedMs)
PARTITION BY toYYYYMM(event_time)
ORDER BY (team_id, event_id, field)
SETTINGS materialize_ttl_recalculate_only = 1, non_replicated_deduplication_window = 1000
