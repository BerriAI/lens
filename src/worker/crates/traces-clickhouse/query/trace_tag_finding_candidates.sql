tag_spans AS (
    SELECT TeamId, ApiKeyHash, UserId, TraceId, SpanId, Timestamp, Duration,
           CallKeys, LiteLLMRequestId, ResourceAttributes, SpanAttributes
    FROM otel_traces
    WHERE has({filter_keys:Array(String)},'tag')
      AND TraceId={trace_id:String}
      AND hex(SHA256(concat(TeamId,char(0),ApiKeyHash,char(0),TraceId)))={trace_ref:String}
      AND ({all_teams:UInt8}=1 OR TeamId={team:String})
      AND ({key_hash:String}='' OR ApiKeyHash={key_hash:String})
),
