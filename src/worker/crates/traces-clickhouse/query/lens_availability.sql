SELECT
    EXISTS(SELECT 1 FROM otel_traces
        WHERE ({all_teams:UInt8}=1 OR TeamId={team:String})
          AND ({key_hash:String}='' OR ApiKeyHash={key_hash:String})
          AND (TeamId, ApiKeyHash, TraceId) NOT IN (SELECT TeamId, ApiKeyHash, TraceId FROM lens_eval_traces
                  WHERE ({all_teams:UInt8}=1 OR TeamId={team:String})
                    AND ({key_hash:String}='' OR ApiKeyHash={key_hash:String}))) AS traces,
    EXISTS(SELECT 1 FROM spend_logs
        WHERE ({all_teams:UInt8}=1 OR team_id={team:String})
          AND ({key_hash:String}='' OR api_key={key_hash:String})
          AND NOT JSONExtractBool(metadata,'litellm_lens_internal')
          AND (team_id,api_key,response_id) NOT IN (
              SELECT TeamId,ApiKeyHash,arrayJoin(RequestIds) FROM lens_eval_traces
              WHERE ({all_teams:UInt8}=1 OR TeamId={team:String})
                AND ({key_hash:String}='' OR ApiKeyHash={key_hash:String}))) AS requests
