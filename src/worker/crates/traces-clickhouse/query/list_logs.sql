SELECT
    RecordId AS record_id,
    toUnixTimestamp64Nano(Timestamp) AS timestamp_ns,
    toUnixTimestamp64Nano(ObservedTimestamp) AS observed_timestamp_ns,
    SeverityNumber AS severity_number,
    SeverityText AS severity_text,
    EventName AS event_name,
    nullIf(SemanticEventName, '') AS semantic_event_name,
    ScopeName AS scope_name,
    ServiceName AS service_name,
    TraceId AS trace_id,
    SpanId AS span_id,
    nullIf(ConversationId, '') AS conversation_id
FROM otel_logs FINAL
WHERE Timestamp >= fromUnixTimestamp64Milli({start_ms:Int64})
  AND Timestamp < fromUnixTimestamp64Milli({end_ms:Int64})
  AND ({service:String} = '' OR ServiceName = {service:String})
  AND ({event:String} = '' OR SemanticEventName = {event:String})
  AND ({severity:String} = '' OR SeverityText = {severity:String})
  AND ({trace_id:String} = '' OR TraceId = {trace_id:String})
  AND ({conversation_id:String} = '' OR ConversationId = {conversation_id:String})
  AND ({has_cursor:UInt8} = 0 OR
       (toUnixTimestamp64Nano(Timestamp), RecordId) <
       ({cursor_timestamp_ns:Int64}, {cursor_record_id:String}))
ORDER BY Timestamp DESC, RecordId DESC
LIMIT {limit:UInt32}
