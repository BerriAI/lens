SELECT any(trace_ref) AS trace_ref,
       any(TeamId) AS team_id,
       any(ApiKeyHash) AS api_key_hash,
       if(uniqExact(tuple(TeamId, ApiKeyHash)) = 1
          AND uniqExactIf(session, session != '') = 1
          AND countIf(ambiguous_session) = 0
          AND countIf(ParentSpanId IN ('', '0000000000000000')) > 0,
          anyIf(session, session != ''), '') AS session_id,
       minIf(start_ns, ParentSpanId IN ('', '0000000000000000')) AS start_ns
FROM (
    SELECT TeamId, ApiKeyHash, SpanId, ParentSpanId,
           hex(SHA256(concat(TeamId, char(0), ApiKeyHash, char(0), TraceId))) AS trace_ref,
           coalesce(nullIf(SpanAttributes['session.id'], ''), ResourceAttributes['session.id']) AS session,
           (SpanAttributes['session.id'] != '' AND ResourceAttributes['session.id'] != ''
            AND SpanAttributes['session.id'] != ResourceAttributes['session.id']) AS ambiguous_session,
           toUnixTimestamp64Nano(Timestamp) AS start_ns
    FROM otel_traces
    WHERE TraceId = {trace_id:String}
      AND EngineReceivedMs <= {snapshot_ms:UInt64}
      AND ({all_teams:UInt8} = 1
           OR ({user_id:String} != '' AND UserId = {user_id:String})
           OR has({team_ids:Array(String)}, TeamId))
      AND ({trace_ref:String} = '' OR trace_ref = {trace_ref:String})
    ORDER BY TeamId, ApiKeyHash, SpanId, EngineReceivedMs DESC,
             Timestamp DESC, ParentSpanId, ambiguous_session DESC, session
    LIMIT 1 BY TeamId, ApiKeyHash, SpanId
)
HAVING count() > 0
