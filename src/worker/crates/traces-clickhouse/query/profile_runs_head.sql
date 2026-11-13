WITH runs AS (
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
profile_candidates AS (SELECT TeamId, ApiKeyHash, TraceId FROM spans),
