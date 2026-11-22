page AS (
    SELECT hex(SHA256(concat(TeamId, char(0), ApiKeyHash, char(0), TraceId))) AS trace_ref,
           if({by_last_activity:UInt8} = 1, toUnixTimestamp64Milli(max(EndTs)), toUnixTimestamp64Milli(min(StartTs))) AS sort_ms
    FROM agent_traces_by_key
    WHERE ({all_teams:UInt8} = 1
           OR ({user_id:String} != '' AND UserIds = [{user_id:String}])
           OR has({team_ids:Array(String)}, TeamId))
      AND ({agent:String} = '' OR (TeamId, ApiKeyHash, TraceId) IN (
          SELECT TeamId, ApiKeyHash, TraceId FROM otel_traces
          WHERE AgentName = {agent:String}
            AND Timestamp >= fromUnixTimestamp64Milli({start_ms:Int64})
      ))
      AND (TeamId, ApiKeyHash, TraceId) NOT IN (
          SELECT TeamId, ApiKeyHash, TraceId FROM lens_eval_traces
          WHERE ({all_teams:UInt8} = 1
                 OR ({user_id:String} != '' AND has(UserIds, {user_id:String}))
                 OR has({team_ids:Array(String)}, TeamId))
          GROUP BY TeamId, ApiKeyHash, TraceId
          HAVING min(StartTs) >= fromUnixTimestamp64Milli({start_ms:Int64}))
      AND (TeamId, ApiKeyHash, TraceId) NOT IN (
          SELECT TeamId, ApiKeyHash, TraceId FROM otel_traces
          WHERE ObservationType = 'llm'
            AND Timestamp >= fromUnixTimestamp64Milli({start_ms:Int64})
          GROUP BY TeamId, ApiKeyHash, TraceId
          HAVING countIf(NOT arrayExists(key -> startsWith(key, 'litellm_request:'), CallKeys)) = 0
             AND countIf((TeamId, ApiKeyHash, LiteLLMRequestId) NOT IN (
                 SELECT TeamId, ApiKeyHash, LiteLLMRequestId FROM otel_traces
                 WHERE ObservationType = 'llm' AND LiteLLMRequestId != ''
                   AND CallEvidence = 'complete'
AND NOT arrayExists(key -> startsWith(key, 'litellm_request:'), CallKeys)
                   AND Timestamp >= fromUnixTimestamp64Milli({start_ms:Int64}))) = 0)
    GROUP BY TeamId, ApiKeyHash, TraceId
    HAVING min(StartTs) >= fromUnixTimestamp64Milli({start_ms:Int64})
       AND min(StartTs) < fromUnixTimestamp64Milli({end_ms:Int64})
       AND ({include_internal:UInt8} = 1 OR sum(LlmCount) + sum(ToolCount) > 0)
       AND ({cursor_ms:Int64} = 0 OR (sort_ms, trace_ref)
            < ({cursor_ms:Int64}, {cursor_trace_id:String}))
    ORDER BY sort_ms DESC, trace_ref DESC
    LIMIT {limit:UInt32}
), selected_refs AS (
    SELECT trace_ref FROM page
),
