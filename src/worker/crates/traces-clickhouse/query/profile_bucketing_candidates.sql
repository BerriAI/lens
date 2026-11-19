SELECT spans.TeamId AS team_id, spans.ApiKeyHash AS api_key_hash, spans.TraceId AS trace_id,
       runs.user_ids AS user_ids, runs.start_ms AS start_ms,
       substringUTF8(spans.input_preview, 1, 1500) AS input_preview,
       failed.attempts AS attempts,
       if(pinned.first_start_ms = 0, runs.start_ms, pinned.first_start_ms) AS assignment_start_ms
FROM spans
INNER JOIN runs ON spans.TeamId = runs.TeamId AND spans.ApiKeyHash = runs.ApiKeyHash
               AND spans.TraceId = runs.TraceId
LEFT JOIN failed ON spans.TeamId = failed.TeamId AND spans.ApiKeyHash = failed.ApiKeyHash
                AND spans.TraceId = failed.TraceId
LEFT JOIN pinned ON spans.TeamId = pinned.TeamId AND spans.ApiKeyHash = pinned.ApiKeyHash
                AND spans.TraceId = pinned.TraceId
ORDER BY start_ms DESC, trace_id
LIMIT {limit:UInt32}
