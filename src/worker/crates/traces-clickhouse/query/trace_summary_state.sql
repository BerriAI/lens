summaries AS (
    SELECT TeamId, ApiKeyHash, TraceId, TraceRef, SourceSpanCount, ComputedAt, SpendKeys, Summary
    FROM lens_trace_summaries FINAL
    WHERE TraceRef IN (SELECT trace_ref FROM selected_refs)
      AND ScopeUserId = if({all_teams:UInt8} = 1 OR has({team_ids:Array(String)}, TeamId), '', {user_id:String})
      AND ({all_teams:UInt8} = 1 OR has({team_ids:Array(String)}, TeamId)
           OR ({user_id:String} != '' AND ScopeUserId = {user_id:String}))
      AND ResolutionVersion = {resolution_version:UInt32}
), source_counts AS (
    SELECT TeamId, ApiKeyHash, TraceId, sum(SpanCount) AS source_span_count
    FROM agent_traces_by_key
    WHERE (TeamId, ApiKeyHash, TraceId) IN (SELECT TeamId, ApiKeyHash, TraceId FROM summaries)
    GROUP BY TeamId, ApiKeyHash, TraceId
), expanded AS (
    SELECT TeamId, TraceRef, ComputedAt, arrayJoin(SpendKeys) AS spend_key
    FROM summaries
), changed_costs AS (
    SELECT DISTINCT expanded.TraceRef
    FROM expanded INNER JOIN (
        SELECT key_kind, key_value, team_id, EngineReceivedMs FROM lens_call_costs
        WHERE (key_kind, key_value, team_id) IN (
            SELECT tupleElement(spend_key, 1), tupleElement(spend_key, 2), TeamId FROM expanded
        )
        UNION ALL
        SELECT key_kind, key_value, team_id, EngineReceivedMs FROM lens_ledger_aliases
        WHERE (key_kind, key_value, team_id) IN (
            SELECT tupleElement(spend_key, 1), tupleElement(spend_key, 2), TeamId FROM expanded
        )
    ) AS costs
        ON costs.team_id = expanded.TeamId
       AND costs.key_kind = tupleElement(expanded.spend_key, 1)
       AND costs.key_value = tupleElement(expanded.spend_key, 2)
    WHERE costs.EngineReceivedMs >= expanded.ComputedAt
), fresh_summaries AS (
SELECT TraceRef AS trace_ref, Summary AS summary
FROM summaries INNER JOIN source_counts USING (TeamId, ApiKeyHash, TraceId)
WHERE summaries.SourceSpanCount = source_counts.source_span_count
  AND TraceRef NOT IN (SELECT TraceRef FROM changed_costs)
)
