tag_spans AS (
    SELECT TeamId, ApiKeyHash, UserId, TraceId, SpanId, Timestamp, Duration,
           CallKeys, LiteLLMRequestId, ResourceAttributes, SpanAttributes
    FROM otel_traces
    WHERE has({filter_keys:Array(String)},'tag')
      AND Timestamp >= fromUnixTimestamp64Milli({start_ms:Int64}) - INTERVAL 7 DAY
      AND Timestamp < fromUnixTimestamp64Milli({end_ms:Int64})
      AND ({all_teams:UInt8}=1 OR has({team_ids:Array(String)},TeamId)
          OR ({user_id:String}!='' AND UserId={user_id:String}))
),
