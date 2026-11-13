1 AS profile_scope
SELECT spans.TeamId AS team_id, spans.ApiKeyHash AS api_key_hash, spans.TraceId AS trace_id,
       runs.start_ms AS start_ms, toUnixTimestamp64Milli(runs.trace_end) AS end_ms,
       trace_profiles.profile_id AS profile_id,
       trace_profiles.category AS category,
       substringUTF8(spans.input_preview, 1, 300) AS input_preview,
       length(spans.agents) AS agent_count
FROM spans
INNER JOIN runs ON spans.TeamId = runs.TeamId AND spans.ApiKeyHash = runs.ApiKeyHash
               AND spans.TraceId = runs.TraceId
INNER JOIN trace_profiles ON spans.TeamId = trace_profiles.TeamId
                         AND spans.ApiKeyHash = trace_profiles.ApiKeyHash
                         AND spans.TraceId = trace_profiles.TraceId
ORDER BY start_ms DESC, trace_id
LIMIT {limit:UInt32}
