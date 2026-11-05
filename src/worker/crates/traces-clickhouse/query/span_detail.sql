SELECT o.SpanId AS span_id,
       if(o.Input = '' AND (o.ObservationType = 'agent' OR o.SpanName = 'session_task.turn'), answer.input, o.Input) AS input,
       if(o.Output = '' AND (o.ObservationType = 'agent' OR o.SpanName = 'session_task.turn'), answer.output, o.Output) AS output,
       o.SpanAttributes AS attributes
FROM otel_traces AS o
LEFT JOIN (
    SELECT TeamId, ApiKeyHash, TraceId,
           argMinIf(
               Input,
               tuple(ObservationType != 'agent', Timestamp),
               ObservationType IN ('agent', 'llm') AND Input != ''
           ) AS input,
           argMaxIf(
               Output,
               tuple(ParentSpanId = {span_id:String} AND ObservationType = 'llm', ParentSpanId = {span_id:String}, Timestamp),
               ObservationType IN ('llm', 'chain') AND Output != ''
           ) AS output
    FROM otel_traces
    WHERE TraceId = {trace_id:String}
      AND ((ObservationType IN ('agent', 'llm') AND Input != '')
           OR (ObservationType IN ('llm', 'chain') AND Output != ''))
      AND ({all_teams:UInt8} = 1
           OR ({user_id:String} != '' AND UserId = {user_id:String})
           OR has({team_ids:Array(String)}, TeamId))
      AND ({trace_ref:String} = '' OR
           hex(SHA256(concat(TeamId, char(0), ApiKeyHash, char(0), TraceId))) = {trace_ref:String})
    GROUP BY TeamId, ApiKeyHash, TraceId
) AS answer ON answer.TraceId = o.TraceId
    AND answer.TeamId = o.TeamId AND answer.ApiKeyHash = o.ApiKeyHash
WHERE o.TraceId = {trace_id:String} AND o.SpanId = {span_id:String}
  AND ({all_teams:UInt8} = 1
       OR ({user_id:String} != '' AND o.UserId = {user_id:String})
       OR has({team_ids:Array(String)}, o.TeamId))
  AND ({trace_ref:String} = '' OR
       hex(SHA256(concat(o.TeamId, char(0), o.ApiKeyHash, char(0), o.TraceId))) = {trace_ref:String})
LIMIT 1
