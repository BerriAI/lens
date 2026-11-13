WITH (span_attributes, resource_attributes) -> coalesce(
    nullIf(trimBoth(span_attributes['task_category']), ''),
    nullIf(trimBoth(resource_attributes['task_category']), ''),
    nullIf(trimBoth(span_attributes['scenario']), ''),
    nullIf(trimBoth(resource_attributes['scenario']), ''), '') AS recorded_profile,
runs AS (
SELECT * FROM (
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
   AND min(StartTs) > now() - toIntervalDay({retention_days:UInt32}) + INTERVAL 1 HOUR
)
WHERE arrayExists((s, e) -> start_ms >= s AND start_ms < e,
                  {window_starts:Array(Int64)}, {window_ends:Array(Int64)})
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
   AND ({require_input:UInt8} = 0 OR input_preview != '')
),
failed AS (
SELECT TeamId, ApiKeyHash, TraceId, max(Attempts) AS attempts
FROM lens_task_profile_assignments
WHERE TaxonomyId = {taxonomy_id:String} AND TaxonomyVersion = {taxonomy_version:UInt32} AND Outcome = 'failed'
  AND (TeamId, ApiKeyHash, TraceId) IN (SELECT TeamId, ApiKeyHash, TraceId FROM spans)
GROUP BY TeamId, ApiKeyHash, TraceId
)
