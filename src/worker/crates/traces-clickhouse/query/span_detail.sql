SELECT o.SpanId AS span_id,
       multiIf(body.input != '', body.input,
               o.Input = '' AND (o.ObservationType = 'agent' OR o.SpanName = 'session_task.turn'), answer.input,
               o.Input) AS input,
       multiIf(body.output != '', body.output,
               o.Output = '' AND (o.ObservationType = 'agent' OR o.SpanName = 'session_task.turn'), answer.output,
               o.Output) AS output,
       o.SpanAttributes AS attributes
FROM otel_traces AS o
LEFT JOIN (
    SELECT TeamId, ApiKeyHash, TraceId,
           argMinIf(
               Input,
               tuple(ObservationType != 'agent', SpanAttributes['lens.codex.prompt_kind'] = 'title', Timestamp),
               ObservationType IN ('agent', 'llm') AND Input != ''
           ) AS input,
           argMaxIf(
               Output,
               tuple(ParentSpanId = {span_id:String} AND ObservationType = 'llm', ParentSpanId = {span_id:String}, Timestamp),
               ObservationType IN ('llm', 'chain') AND Output != ''
               AND (ParentSpanId = {span_id:String} OR roots = 1)
           ) AS output
    FROM (
        SELECT TeamId, ApiKeyHash, TraceId, UserId, ParentSpanId, ObservationType, Input, Output,
               SpanAttributes, Timestamp,
               uniqExactIf(SpanId, ParentSpanId IN ('', '0000000000000000'))
                      OVER (PARTITION BY TeamId, ApiKeyHash, TraceId) AS roots
        FROM otel_traces
        WHERE TraceId = {trace_id:String}
    )
    WHERE ((ObservationType IN ('agent', 'llm') AND Input != '')
           OR (ObservationType IN ('llm', 'chain') AND Output != ''))
      AND ({all_teams:UInt8} = 1
           OR ({user_id:String} != '' AND UserId = {user_id:String})
           OR has({team_ids:Array(String)}, TeamId))
      AND ({trace_ref:String} = '' OR
           hex(SHA256(concat(TeamId, char(0), ApiKeyHash, char(0), TraceId))) = {trace_ref:String})
    GROUP BY TeamId, ApiKeyHash, TraceId
) AS answer ON answer.TraceId = o.TraceId
    AND answer.TeamId = o.TeamId AND answer.ApiKeyHash = o.ApiKeyHash
LEFT JOIN (
    SELECT response.TeamId AS TeamId, response.ApiKeyHash AS ApiKeyHash, response.TraceId AS TraceId,
           response.request_id AS request_id,
           any(request.Input) AS input, any(response.Output) AS output
    FROM (
        SELECT TeamId, ApiKeyHash, TraceId, Output,
               SpanAttributes['request_id'] AS request_id,
               SpanAttributes['request_body_id'] AS request_body_id
        FROM otel_traces
        WHERE TraceId = {trace_id:String} AND SpanName = 'claude_code.api_response_body'
          AND SpanAttributes['request_id'] != ''
    ) AS response
    LEFT JOIN (
        SELECT TeamId, ApiKeyHash, TraceId, Input,
               SpanAttributes['request_body_id'] AS request_body_id
        FROM otel_traces
        WHERE TraceId = {trace_id:String} AND SpanName = 'claude_code.api_request_body'
          AND SpanAttributes['request_body_id'] != ''
    ) AS request ON request.TeamId = response.TeamId AND request.ApiKeyHash = response.ApiKeyHash
        AND request.TraceId = response.TraceId AND request.request_body_id = response.request_body_id
    GROUP BY TeamId, ApiKeyHash, TraceId, request_id
) AS body ON o.SpanName = 'claude_code.llm_request'
    AND body.TraceId = o.TraceId AND body.TeamId = o.TeamId AND body.ApiKeyHash = o.ApiKeyHash
    AND body.request_id = o.SpanAttributes['request_id']
WHERE o.TraceId = {trace_id:String} AND o.SpanId = {span_id:String}
  AND ({all_teams:UInt8} = 1
       OR ({user_id:String} != '' AND o.UserId = {user_id:String})
       OR has({team_ids:Array(String)}, o.TeamId))
  AND ({trace_ref:String} = '' OR
       hex(SHA256(concat(o.TeamId, char(0), o.ApiKeyHash, char(0), o.TraceId))) = {trace_ref:String})
ORDER BY (o.SpanAttributes['lens.codex.root'] = '1' AND o.SpanAttributes['event.name'] = 'codex.user_prompt'),
         (o.SpanAttributes['lens.codex.root'] = '1' AND o.SpanAttributes['lens.codex.prompt_kind'] = 'title'),
         if(o.SpanAttributes['lens.codex.root'] = '1', o.Duration, 0) DESC,
         o.EngineReceivedMs DESC
LIMIT 1
