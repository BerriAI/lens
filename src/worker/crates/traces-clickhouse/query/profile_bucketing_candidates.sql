SELECT spans.TeamId AS team_id, spans.ApiKeyHash AS api_key_hash, spans.TraceId AS trace_id,
       runs.user_ids AS user_ids, runs.start_ms AS start_ms,
       substringUTF8(spans.input_preview, 1, 1500) AS input_preview,
       failed.attempts AS attempts
FROM spans
INNER JOIN runs ON spans.TeamId = runs.TeamId AND spans.ApiKeyHash = runs.ApiKeyHash
               AND spans.TraceId = runs.TraceId
LEFT JOIN failed ON spans.TeamId = failed.TeamId AND spans.ApiKeyHash = failed.ApiKeyHash
                AND spans.TraceId = failed.TraceId
ORDER BY start_ms DESC, trace_id
LIMIT {limit:UInt32}
