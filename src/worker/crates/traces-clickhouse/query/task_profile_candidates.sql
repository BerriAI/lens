SELECT TeamId AS team_id, ApiKeyHash AS api_key_hash, TraceId AS trace_id,
       arrayElement(arraySort(groupUniqArrayArray(AgentNames)), 1) AS agent_name,
       any(ServiceName) AS service_name,
       groupUniqArrayArray(UserIds) AS user_ids,
       toUnixTimestamp64Milli(min(StartTs)) AS start_ms
FROM agent_traces_by_key
WHERE (TeamId, ApiKeyHash, TraceId) NOT IN
      (SELECT TeamId, ApiKeyHash, TraceId FROM lens_task_profiles WHERE ResolutionVersion = {version:UInt32})
  AND (TeamId, ApiKeyHash, TraceId) NOT IN
      (SELECT TeamId, ApiKeyHash, TraceId FROM lens_eval_traces)
GROUP BY TeamId, ApiKeyHash, TraceId
HAVING max(EndTs) < now() - INTERVAL 30 SECOND
ORDER BY start_ms DESC
LIMIT 1
