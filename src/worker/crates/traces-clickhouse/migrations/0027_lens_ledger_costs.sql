CREATE TABLE IF NOT EXISTS {database}.lens_ledger_costs
(
    event_id String,
    team_id LowCardinality(String),
    event_time DateTime64(3),
    request_id String,
    litellm_call_id String,
    response_id String,
    upstream_response_id String,
    provider_request_id String,
    trace_id String,
    span_id String,
    session_id String,
    api_key String,
    user String,
    organization_id String,
    model LowCardinality(String),
    custom_llm_provider LowCardinality(String),
    call_type LowCardinality(String),
    status LowCardinality(String),
    spend Nullable(Float64) DEFAULT NULL,
    prompt_tokens UInt32,
    completion_tokens UInt32,
    EngineReceivedMs UInt64 DEFAULT 0
)
ENGINE = ReplacingMergeTree(EngineReceivedMs)
PARTITION BY toYYYYMM(event_time)
ORDER BY (toDate(event_time), team_id, event_id)
SETTINGS materialize_ttl_recalculate_only = 1, non_replicated_deduplication_window = 1000
