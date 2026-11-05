CREATE TABLE IF NOT EXISTS {database}.otel_logs
(
    Timestamp DateTime64(9) CODEC(Delta, ZSTD(1)),
    ObservedTimestamp DateTime64(9) CODEC(Delta, ZSTD(1)),
    RecordId FixedString(64) CODEC(ZSTD(1)),
    TeamId LowCardinality(String),
    ApiKeyHash String,
    OrgId LowCardinality(String),
    UserId LowCardinality(String),
    ServiceName LowCardinality(String),
    ConversationId String CODEC(ZSTD(1)),
    TraceId String CODEC(ZSTD(1)),
    SpanId String CODEC(ZSTD(1)),
    TraceFlags UInt32,
    ResourceAttributes String CODEC(ZSTD(3)),
    ResourceSchemaUrl String,
    ScopeName String CODEC(ZSTD(1)),
    ScopeVersion String,
    ScopeAttributes String CODEC(ZSTD(3)),
    ScopeSchemaUrl String,
    EventName String CODEC(ZSTD(1)),
    SemanticEventName LowCardinality(String),
    Framework LowCardinality(String),
    CodexProjection String CODEC(ZSTD(3)),
    SeverityNumber Int32,
    SeverityText LowCardinality(String),
    Body String CODEC(ZSTD(3)),
    BodyPresent Bool,
    Attributes String CODEC(ZSTD(3)),
    DroppedAttributesCount UInt32,
    EngineReceivedMs UInt64 DEFAULT 0,
    INDEX idx_conversation ConversationId TYPE bloom_filter(0.01) GRANULARITY 1,
    INDEX idx_trace TraceId TYPE bloom_filter(0.001) GRANULARITY 1,
    INDEX idx_event SemanticEventName TYPE set(256) GRANULARITY 1,
    INDEX idx_service ServiceName TYPE set(256) GRANULARITY 1
)
ENGINE = ReplacingMergeTree(EngineReceivedMs)
PARTITION BY toDate(Timestamp)
ORDER BY (TeamId, ApiKeyHash, Timestamp, RecordId)
SETTINGS ttl_only_drop_parts = 1, materialize_ttl_recalculate_only = 1, non_replicated_deduplication_window = 1000
