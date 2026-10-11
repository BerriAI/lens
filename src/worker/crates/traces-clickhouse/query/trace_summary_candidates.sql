WITH runs AS (
    SELECT TeamId, ApiKeyHash, TraceId, sum(SpanCount) AS source_span_count,
           toUnixTimestamp64Milli(min(StartTs)) AS start_ms
    FROM agent_traces_by_key
    WHERE (TeamId, ApiKeyHash, TraceId) NOT IN (SELECT TeamId, ApiKeyHash, TraceId FROM lens_eval_traces)
    GROUP BY TeamId, ApiKeyHash, TraceId
), summaries AS (
    SELECT TeamId, ApiKeyHash, TraceId, SourceSpanCount, ComputedAt, ResolutionVersion, SpendKeys
    FROM lens_trace_summaries FINAL
    WHERE ScopeUserId = ''
), expanded AS (
    SELECT TeamId, ApiKeyHash, TraceId, ComputedAt, arrayJoin(SpendKeys) AS spend_key
    FROM summaries
), changed_costs AS (
    SELECT DISTINCT expanded.TeamId, expanded.ApiKeyHash, expanded.TraceId
    FROM expanded INNER JOIN lens_call_costs AS costs
        ON costs.team_id = expanded.TeamId
       AND costs.key_kind = tupleElement(expanded.spend_key, 1)
       AND costs.key_value = tupleElement(expanded.spend_key, 2)
    WHERE costs.EngineReceivedMs >= expanded.ComputedAt
)
SELECT runs.TeamId AS team_id, runs.ApiKeyHash AS api_key_hash, runs.TraceId AS trace_id,
       hex(SHA256(concat(runs.TeamId, char(0), runs.ApiKeyHash, char(0), runs.TraceId))) AS trace_ref,
       runs.source_span_count, runs.start_ms
FROM runs
LEFT JOIN summaries USING (TeamId, ApiKeyHash, TraceId)
WHERE (summaries.SourceSpanCount != runs.source_span_count
   OR summaries.ResolutionVersion != {resolution_version:UInt32}
   OR (runs.TeamId, runs.ApiKeyHash, runs.TraceId) IN (SELECT TeamId, ApiKeyHash, TraceId FROM changed_costs))
  AND (summaries.ResolutionVersion != 0
       OR summaries.ComputedAt <= {retry_before_ms:UInt64})
ORDER BY summaries.ComputedAt, runs.start_ms DESC, trace_ref
LIMIT {limit:UInt32}
