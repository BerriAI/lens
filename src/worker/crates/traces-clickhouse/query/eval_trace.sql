WITH identities AS (
    SELECT DISTINCT TraceId, ApiKeyHash
    FROM otel_traces
    WHERE TeamId={team:String}
      AND (({attribute:String}='trace_id' AND TraceId={value:String})
        OR ({attribute:String}='session.id' AND
          (SpanAttributes['session.id']={value:String} OR ResourceAttributes['session.id']={value:String})))
)
SELECT TraceId AS trace_id,
    ApiKeyHash AS api_key_hash,
    hex(SHA256(concat(TeamId,char(0),ApiKeyHash,char(0),TraceId))) AS trace_ref,
    SpanId AS span_id, ParentSpanId AS parent_span_id, SpanName AS name, ObservationType AS kind,
    SpanAttributes['gen_ai.tool.name'] AS tool_name,
    toUnixTimestamp64Nano(Timestamp) AS start_ns,
    toUnixTimestamp64Nano(addNanoseconds(Timestamp,Duration)) AS end_ns,
    EngineReceivedMs AS received_ms, StatusCode AS status,
    if(SpanAttributes['agent.version']!='',SpanAttributes['agent.version'],ResourceAttributes['agent.version']) AS version,
    Input AS input, Output AS output,
    concat(trace_ref,char(0),SpanId) AS key
FROM otel_traces
WHERE TeamId={team:String} AND (TraceId,ApiKeyHash) IN identities
  AND key>{cursor:String}
ORDER BY key, EngineReceivedMs DESC
LIMIT 1 BY key
LIMIT 1000
