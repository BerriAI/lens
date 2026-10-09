SELECT DISTINCT AgentName AS agent_name
FROM otel_traces
WHERE AgentName != ''
  AND ({all_teams:UInt8}=1 OR TeamId={team:String})
  AND ({key_hash:String}='' OR ApiKeyHash={key_hash:String})
  AND (TeamId, ApiKeyHash, TraceId) NOT IN (SELECT TeamId, ApiKeyHash, TraceId FROM lens_eval_traces
      WHERE ({all_teams:UInt8}=1 OR TeamId={team:String})
        AND ({key_hash:String}='' OR ApiKeyHash={key_hash:String}))
ORDER BY agent_name
