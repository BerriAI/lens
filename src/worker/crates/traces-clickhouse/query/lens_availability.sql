WITH eval_traces AS (
    SELECT TeamId, ApiKeyHash, TraceId FROM otel_traces
    WHERE ({all_teams:UInt8}=1 OR TeamId={team:String})
      AND ({key_hash:String}='' OR ApiKeyHash={key_hash:String})
      AND coalesce(nullIf(SpanAttributes['deployment.environment'], ''),
          ResourceAttributes['deployment.environment']) = 'lens-eval'
)
SELECT
    EXISTS(SELECT 1 FROM otel_traces
        WHERE ({all_teams:UInt8}=1 OR TeamId={team:String})
          AND (TeamId,ApiKeyHash,TraceId) NOT IN eval_traces
          AND ({key_hash:String}='' OR ApiKeyHash={key_hash:String})) AS traces,
    EXISTS(SELECT 1 FROM spend_logs
        WHERE ({all_teams:UInt8}=1 OR team_id={team:String})
          AND (team_id,api_key,trace_id) NOT IN eval_traces
          AND coalesce(nullIf(JSONExtractString(metadata,'deployment.environment'),''),
              JSONExtractString(metadata,'requester_metadata','deployment.environment')) != 'lens-eval'
          AND (team_id,api_key,response_id) NOT IN (
              SELECT TeamId,ApiKeyHash,LiteLLMRequestId FROM otel_traces
              WHERE LiteLLMRequestId!='' AND (TeamId,ApiKeyHash,TraceId) IN eval_traces
          )
          AND ({key_hash:String}='' OR api_key={key_hash:String})
          AND NOT JSONExtractBool(metadata,'litellm_lens_internal')) AS requests
