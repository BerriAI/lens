WITH (span_attributes, resource_attributes) -> coalesce(
    nullIf(trimBoth(span_attributes['task_category']), ''),
    nullIf(trimBoth(resource_attributes['task_category']), ''),
    nullIf(trimBoth(span_attributes['scenario']), ''),
    nullIf(trimBoth(resource_attributes['scenario']), ''), '') AS recorded_profile,
runs AS (
SELECT TeamId, ApiKeyHash, TraceId,
       toUnixTimestamp64Milli(min(StartTs)) AS start_ms,
       min(StartTs) AS trace_start, max(EndTs) AS trace_end
FROM agent_traces_by_key
WHERE ({all_teams:UInt8} = 1
       OR ({user_id:String} != '' AND UserIds = [{user_id:String}])
       OR has({team_ids:Array(String)}, TeamId))
  AND (TeamId, ApiKeyHash, TraceId) NOT IN (SELECT TeamId, ApiKeyHash, TraceId FROM lens_eval_traces
      WHERE ({all_teams:UInt8} = 1
             OR ({user_id:String} != '' AND has(UserIds, {user_id:String}))
             OR has({team_ids:Array(String)}, TeamId))
      GROUP BY TeamId, ApiKeyHash, TraceId
      HAVING min(StartTs) >= fromUnixTimestamp64Milli({start_ms:Int64}))
GROUP BY TeamId, ApiKeyHash, TraceId
HAVING min(StartTs) >= fromUnixTimestamp64Milli({start_ms:Int64})
   AND min(StartTs) < fromUnixTimestamp64Milli({end_ms:Int64})
),
spans AS (
SELECT TeamId, ApiKeyHash, TraceId,
       argMinIf(recorded_profile(SpanAttributes, ResourceAttributes),
                tuple(if(ParentSpanId = '', 0, 1), Timestamp, SpanId),
                recorded_profile(SpanAttributes, ResourceAttributes) != '') AS recorded,
       argMinIf(InputPreview, tuple(if(ParentSpanId = '', 0, 1), Timestamp, SpanId),
                InputPreview != '') AS input_preview,
       groupUniqArrayIf(AgentName, AgentName != '') AS agents
FROM otel_traces
WHERE Timestamp >= (SELECT min(trace_start) FROM runs)
  AND Timestamp <= (SELECT max(trace_end) FROM runs)
  AND (TeamId, ApiKeyHash, TraceId) IN (SELECT TeamId, ApiKeyHash, TraceId FROM runs)
GROUP BY TeamId, ApiKeyHash, TraceId
HAVING has(agents, {agent:String})
),
inferred AS (
SELECT TeamId, ApiKeyHash, TraceId,
       argMax(ClusterId, ComputedAt) AS cluster_id, argMax(Category, ComputedAt) AS cluster_label
FROM lens_task_profiles
WHERE ResolutionVersion = 1
  AND (TeamId, ApiKeyHash, TraceId) IN (SELECT TeamId, ApiKeyHash, TraceId FROM spans)
GROUP BY TeamId, ApiKeyHash, TraceId
),
enhanced AS (
SELECT TeamId, ApiKeyHash, TraceId,
       argMax(TaskId, ComputedAt) AS task_id, argMax(Outcome, ComputedAt) AS outcome
FROM lens_task_profile_assignments
WHERE TaxonomyId = {taxonomy_id:String}
  AND TaxonomyVersion = {taxonomy_version:UInt32}
  AND (TeamId, ApiKeyHash, TraceId) IN (SELECT TeamId, ApiKeyHash, TraceId FROM spans)
GROUP BY TeamId, ApiKeyHash, TraceId
)
SELECT spans.TeamId AS team_id, spans.ApiKeyHash AS api_key_hash, spans.TraceId AS trace_id,
       runs.start_ms AS start_ms, spans.recorded AS recorded,
       inferred.cluster_id AS cluster_id, inferred.cluster_label AS cluster_label,
       enhanced.task_id AS task_id, enhanced.outcome AS outcome,
       substringUTF8(spans.input_preview, 1, 300) AS input_preview,
       length(spans.agents) AS agent_count
FROM spans
INNER JOIN runs ON spans.TeamId = runs.TeamId AND spans.ApiKeyHash = runs.ApiKeyHash
               AND spans.TraceId = runs.TraceId
LEFT JOIN inferred ON spans.TeamId = inferred.TeamId AND spans.ApiKeyHash = inferred.ApiKeyHash
                  AND spans.TraceId = inferred.TraceId
LEFT JOIN enhanced ON spans.TeamId = enhanced.TeamId AND spans.ApiKeyHash = enhanced.ApiKeyHash
                  AND spans.TraceId = enhanced.TraceId
ORDER BY start_ms DESC, trace_id
LIMIT {limit:UInt32}
