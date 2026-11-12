WITH (span_attributes, resource_attributes) -> coalesce(
    nullIf(trimBoth(span_attributes['task_category']), ''),
    nullIf(trimBoth(resource_attributes['task_category']), ''),
    nullIf(trimBoth(span_attributes['scenario']), ''),
    nullIf(trimBoth(resource_attributes['scenario']), ''), '') AS recorded_profile,
runs AS (
SELECT TeamId, ApiKeyHash, TraceId, groupUniqArrayArray(UserIds) AS user_ids,
       toUnixTimestamp64Milli(min(StartTs)) AS start_ms,
       min(StartTs) AS trace_start, max(EndTs) AS trace_end
FROM agent_traces_by_key
WHERE (TeamId, ApiKeyHash, TraceId) NOT IN (SELECT TeamId, ApiKeyHash, TraceId FROM lens_eval_traces)
  AND (TeamId, ApiKeyHash, TraceId) NOT IN (
      SELECT TeamId, ApiKeyHash, TraceId FROM lens_task_profile_assignments
      WHERE TaxonomyId = {taxonomy_id:String} AND TaxonomyVersion = {taxonomy_version:UInt32}
        AND (Outcome IN ('assigned', 'unknown')
             OR (Outcome = 'failed' AND (Attempts >= {max_attempts:UInt8}
                 OR ComputedAt > toUInt64(toUnixTimestamp64Milli(now64(3))) - 600000))))
GROUP BY TeamId, ApiKeyHash, TraceId
HAVING min(StartTs) >= fromUnixTimestamp64Milli({start_ms:Int64})
   AND min(StartTs) < fromUnixTimestamp64Milli({end_ms:Int64})
   AND max(EndTs) < now() - INTERVAL 30 SECOND
),
spans AS (
SELECT TeamId, ApiKeyHash, TraceId,
       groupUniqArrayIf(AgentName, AgentName != '') AS agents,
       countIf(recorded_profile(SpanAttributes, ResourceAttributes) != '') AS recorded_spans,
       argMinIf(InputPreview, tuple(if(ParentSpanId = '', 0, 1), Timestamp, SpanId),
                InputPreview != '') AS input_preview
FROM otel_traces
WHERE Timestamp >= (SELECT min(trace_start) FROM runs)
  AND Timestamp <= (SELECT max(trace_end) FROM runs)
  AND (TeamId, ApiKeyHash, TraceId) IN (SELECT TeamId, ApiKeyHash, TraceId FROM runs)
GROUP BY TeamId, ApiKeyHash, TraceId
HAVING has(agents, {agent:String}) AND recorded_spans = 0
),
failed AS (
SELECT TeamId, ApiKeyHash, TraceId, max(Attempts) AS attempts
FROM lens_task_profile_assignments
WHERE TaxonomyId = {taxonomy_id:String} AND TaxonomyVersion = {taxonomy_version:UInt32} AND Outcome = 'failed'
  AND (TeamId, ApiKeyHash, TraceId) IN (SELECT TeamId, ApiKeyHash, TraceId FROM spans)
GROUP BY TeamId, ApiKeyHash, TraceId
)
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
