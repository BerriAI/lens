SELECT
    RecordId AS record_id,
    toUnixTimestamp64Nano(Timestamp) AS timestamp_ns,
    toUnixTimestamp64Nano(RawTimestamp) AS raw_timestamp_ns,
    toUnixTimestamp64Nano(ObservedTimestamp) AS observed_timestamp_ns,
    SeverityNumber AS severity_number,
    SeverityText AS severity_text,
    EventName AS event_name,
    nullIf(SemanticEventName, '') AS semantic_event_name,
    Body AS body,
    BodyPresent AS body_present,
    Attributes AS attributes,
    ResourceAttributes AS resource_attributes,
    ResourceSchemaUrl AS resource_schema_url,
    ScopeName AS scope_name,
    ScopeVersion AS scope_version,
    ScopeAttributes AS scope_attributes,
    ScopeSchemaUrl AS scope_schema_url,
    TraceId AS trace_id,
    SpanId AS span_id,
    TraceFlags AS trace_flags,
    DroppedAttributesCount AS dropped_attributes_count,
    ServiceName AS service_name,
    nullIf(ConversationId, '') AS conversation_id,
    CodexProjection AS codex_projection
FROM otel_logs FINAL
WHERE RecordId = {record_id:String}
LIMIT 1
