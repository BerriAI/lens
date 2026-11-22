SELECT SpanId AS span_id, TeamId AS team_id, ApiKeyHash AS api_key, ScopeName AS scope, SpanName AS name,
       ParentSpanId AS parent_span_id, ResourceAttributes AS resource_attributes, StatusCode AS status_code,
       StatusMessage AS status_message, SpanAttributes AS attributes
FROM otel_traces
WHERE ({all_teams:UInt8}=1 OR TeamId={team:String})
  AND ({key_hash:String}='' OR ApiKeyHash={key_hash:String})
  AND Timestamp >= parseDateTime64BestEffortOrZero({start_time:String}, 9) - INTERVAL 7 DAY
  AND ({trace_ref:String}='' OR hex(SHA256(concat(TeamId, char(0), ApiKeyHash, char(0), TraceId)))={trace_ref:String})
  AND TraceId={id:String} AND TeamId={record_team:String} AND has({span_ids:Array(String)}, SpanId)
  AND SpanAttributes['litellm.lens.content'] != ''
  AND (TeamId, ApiKeyHash, TraceId) NOT IN (SELECT TeamId, ApiKeyHash, TraceId FROM lens_eval_traces
      WHERE TeamId={record_team:String} AND TraceId={id:String})
ORDER BY SpanId LIMIT 1 BY SpanId LIMIT 40
