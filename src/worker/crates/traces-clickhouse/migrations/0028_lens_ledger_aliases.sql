CREATE TABLE IF NOT EXISTS {database}.lens_ledger_aliases
(
    key_kind LowCardinality(String),
    key_value String,
    team_id LowCardinality(String),
    event_time DateTime64(3),
    event_id String,
    api_key String,
    user String,
    EngineReceivedMs UInt64 DEFAULT 0
)
ENGINE = ReplacingMergeTree(EngineReceivedMs)
PARTITION BY toYYYYMM(event_time)
ORDER BY (key_kind, key_value, team_id, event_time, event_id)
SETTINGS materialize_ttl_recalculate_only = 1, non_replicated_deduplication_window = 1000
