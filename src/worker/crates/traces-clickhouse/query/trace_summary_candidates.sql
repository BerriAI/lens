WITH selected_keys AS (
    SELECT tupleElement(key, 1) AS TeamId, tupleElement(key, 2) AS ApiKeyHash,
           tupleElement(key, 3) AS TraceId
    FROM (SELECT arrayJoin(JSONExtract({keys:String}, 'Array(Tuple(String, String, String))')) AS key)
), runs AS (
    SELECT TeamId, ApiKeyHash, TraceId, sum(SpanCount) AS source_span_count,
           toUnixTimestamp64Milli(min(StartTs)) AS start_ms
    FROM agent_traces_by_key
    WHERE (TeamId, ApiKeyHash, TraceId) IN (SELECT * FROM selected_keys)
      AND (TeamId, ApiKeyHash, TraceId) NOT IN (
          SELECT TeamId, ApiKeyHash, TraceId FROM lens_eval_traces
          WHERE (TeamId, ApiKeyHash, TraceId) IN (SELECT * FROM selected_keys)
      )
    GROUP BY TeamId, ApiKeyHash, TraceId
), summaries AS (
    SELECT TeamId, ApiKeyHash, TraceId, SourceSpanCount, ComputedAt, ResolutionVersion, SpendKeys
    FROM lens_trace_summaries FINAL
    WHERE TraceRef IN {trace_refs:Array(String)} AND ScopeUserId = ''
), expanded AS (
    SELECT TeamId, ApiKeyHash, TraceId, ComputedAt, arrayJoin(SpendKeys) AS spend_key
    FROM summaries
), changed_costs AS (
    SELECT DISTINCT expanded.TeamId, expanded.ApiKeyHash, expanded.TraceId
    FROM expanded INNER JOIN (
        SELECT * FROM lens_call_costs
        WHERE (key_kind, key_value, team_id) IN (
            SELECT tupleElement(spend_key, 1), tupleElement(spend_key, 2), TeamId FROM expanded
        )
    ) AS costs
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
ORDER BY runs.TeamId, runs.ApiKeyHash, runs.TraceId
LIMIT {limit:UInt32}
