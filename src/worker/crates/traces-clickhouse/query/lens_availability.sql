SELECT
    EXISTS(SELECT 1 FROM otel_traces
        WHERE ({all_teams:UInt8}=1 OR TeamId={team:String})
          AND ({key_hash:String}='' OR ApiKeyHash={key_hash:String})
          AND (TeamId, ApiKeyHash, TraceId) NOT IN (SELECT TeamId, ApiKeyHash, TraceId FROM lens_eval_traces
                  WHERE ({all_teams:UInt8}=1 OR TeamId={team:String})
                    AND ({key_hash:String}='' OR ApiKeyHash={key_hash:String}))) AS traces,
    toUInt8(0) AS requests
