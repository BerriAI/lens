SELECT o.TeamId AS team_id,
       o.ApiKeyHash AS api_key,
       o.ScopeName AS scope,
       o.SpanName AS name,
       o.ParentSpanId AS parent_span_id,
       o.ResourceAttributes AS resource_attributes,
       o.StatusCode AS status_code,
       o.StatusMessage AS status_message
FROM otel_traces AS o
WHERE o.TraceId = {trace_id:String} AND o.SpanId = {span_id:String}
  AND o.SpanAttributes['litellm.lens.content'] != ''
  AND ({all_teams:UInt8} = 1
       OR ({user_id:String} != '' AND o.UserId = {user_id:String})
       OR has({team_ids:Array(String)}, o.TeamId))
  AND ({trace_ref:String} = '' OR
       hex(SHA256(concat(o.TeamId, char(0), o.ApiKeyHash, char(0), o.TraceId))) = {trace_ref:String})
ORDER BY o.EngineReceivedMs DESC
LIMIT 1
