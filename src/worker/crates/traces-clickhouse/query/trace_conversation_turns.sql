WITH conversation_candidates AS (
    SELECT DISTINCT TraceId, SpanId
    FROM otel_traces
    WHERE TeamId = {team_id:String} AND ApiKeyHash = {api_key_hash:String}
      AND TraceId != {current_trace_id:String}
      AND EngineReceivedMs <= {snapshot_ms:UInt64}
      AND ({all_teams:UInt8} = 1
           OR ({user_id:String} != '' AND UserId = {user_id:String})
           OR has({team_ids:Array(String)}, TeamId))
      AND ParentSpanId IN ('', '0000000000000000')
      AND {session_id:String} != ''
      AND coalesce(nullIf(SpanAttributes['session.id'], ''), ResourceAttributes['session.id']) = {session_id:String}
      AND Timestamp < fromUnixTimestamp64Nano({before_ns:Int64})
)
SELECT TraceId AS trace_id, trace_ref, SpanId AS span_id, start_ns,
       Input AS input,
       if(toInt128(start_ns) + toInt128(Duration) <= toInt128({before_ns:Int64}), Output, '') AS output
FROM (
    SELECT TraceId, SpanId, ParentSpanId, Input, Output, Duration,
           hex(SHA256(concat(TeamId, char(0), ApiKeyHash, char(0), TraceId))) AS trace_ref,
           coalesce(nullIf(SpanAttributes['session.id'], ''), ResourceAttributes['session.id']) AS session,
           (SpanAttributes['session.id'] != '' AND ResourceAttributes['session.id'] != ''
            AND SpanAttributes['session.id'] != ResourceAttributes['session.id']) AS ambiguous_session,
           toUnixTimestamp64Nano(Timestamp) AS start_ns
    FROM otel_traces
    WHERE TeamId = {team_id:String} AND ApiKeyHash = {api_key_hash:String}
      AND TraceId != {current_trace_id:String}
      AND EngineReceivedMs <= {snapshot_ms:UInt64}
      AND (TraceId, SpanId) IN (SELECT TraceId, SpanId FROM conversation_candidates)
      AND ({all_teams:UInt8} = 1
           OR ({user_id:String} != '' AND UserId = {user_id:String})
           OR has({team_ids:Array(String)}, TeamId))
    ORDER BY TraceId, SpanId, EngineReceivedMs DESC,
             Timestamp DESC, ParentSpanId, ambiguous_session DESC, session,
             Input DESC, Output DESC, Duration DESC
    LIMIT 1 BY TraceId, SpanId
)
WHERE ParentSpanId IN ('', '0000000000000000')
  AND NOT ambiguous_session
  AND {session_id:String} != '' AND session = {session_id:String}
  AND start_ns < {before_ns:Int64}
  AND ({has_cursor:UInt8} = 0 OR
       (start_ns, trace_ref, SpanId) > ({after_start_ns:Int64}, {after_trace_ref:String}, {after_span_id:String}))
ORDER BY start_ns, trace_ref, SpanId
LIMIT least({limit:UInt32}, 51)
