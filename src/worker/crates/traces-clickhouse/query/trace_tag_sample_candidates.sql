tag_spans AS (
    SELECT TeamId, ApiKeyHash, UserId, TraceId, SpanId, Timestamp, Duration,
           CallKeys, LiteLLMRequestId, ResourceAttributes, SpanAttributes
    FROM otel_traces
    WHERE has({filter_keys:Array(String)},'tag')
      AND Timestamp >= fromUnixTimestamp64Milli(toInt64({start:UInt64})) - INTERVAL 7 DAY
      AND Timestamp < fromUnixTimestamp64Milli(toInt64({end:UInt64}))
      AND ({all_teams:UInt8}=1 OR TeamId={team:String})
      AND ({key_hash:String}='' OR ApiKeyHash={key_hash:String})
),
