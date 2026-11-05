  AND ({agent:String} = '' OR (TeamId, ApiKeyHash, TraceId) IN (
      SELECT TeamId, ApiKeyHash, TraceId FROM otel_traces
      WHERE AgentName = {agent:String}
        AND Timestamp >= fromUnixTimestamp64Milli({start_ms:Int64})
  ))
  AND (TeamId, ApiKeyHash, TraceId) NOT IN (SELECT TeamId, ApiKeyHash, TraceId FROM lens_eval_traces
      WHERE ({all_teams:UInt8} = 1
             OR ({user_id:String} != '' AND has(UserIds, {user_id:String}))
             OR has({team_ids:Array(String)}, TeamId))
      GROUP BY TeamId, ApiKeyHash, TraceId
      HAVING min(StartTs) >= fromUnixTimestamp64Milli({start_ms:Int64}))
GROUP BY TeamId, ApiKeyHash, TraceId
HAVING min(StartTs) >= fromUnixTimestamp64Milli({start_ms:Int64})
   AND min(StartTs) < fromUnixTimestamp64Milli({end_ms:Int64})
   AND ({cursor_ms:Int64} = 0 OR (toUnixTimestamp64Milli(min(StartTs)), trace_ref)
        < ({cursor_ms:Int64}, {cursor_trace_id:String}))
ORDER BY start_ms DESC, trace_ref DESC
LIMIT {limit:UInt32}
)
SELECT page.*,
       identities.agent_names AS agent_names, identities.agent_count AS agent_count,
       identities.frameworks AS frameworks
FROM page
LEFT JOIN (
    SELECT TeamId, ApiKeyHash, TraceId,
           arraySort(groupUniqArrayIf(AgentName, AgentName != '')) AS agent_names,
           arraySort(groupUniqArrayIf(toString(Framework), Framework != '')) AS frameworks,
           uniqExactIf(if(AgentName = '', SpanName, AgentName), ObservationType = 'agent') AS agent_count
    FROM otel_traces
    WHERE Timestamp >= fromUnixTimestamp64Milli({start_ms:Int64})
      AND TraceId IN (SELECT trace_id FROM page)
    GROUP BY TeamId, ApiKeyHash, TraceId
) AS identities
ON page.team_id = identities.TeamId AND page.api_key_hash = identities.ApiKeyHash
   AND page.trace_id = identities.TraceId
ORDER BY page.start_ms DESC, page.trace_ref DESC
