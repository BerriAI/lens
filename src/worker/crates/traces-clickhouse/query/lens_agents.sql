WITH eval_traces AS (
    SELECT TeamId, ApiKeyHash, TraceId FROM otel_traces
    WHERE ({all_teams:UInt8}=1 OR TeamId={team:String})
      AND ({key_hash:String}='' OR ApiKeyHash={key_hash:String})
      AND coalesce(nullIf(SpanAttributes['deployment.environment'], ''),
          ResourceAttributes['deployment.environment']) = 'lens-eval'
)
SELECT DISTINCT AgentName AS agent_name
FROM otel_traces
WHERE AgentName != ''
  AND (TeamId,ApiKeyHash,TraceId) NOT IN eval_traces
  AND ({all_teams:UInt8}=1 OR TeamId={team:String})
  AND ({key_hash:String}='' OR ApiKeyHash={key_hash:String})
ORDER BY agent_name
