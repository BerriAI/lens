profile_candidates AS (
    SELECT DISTINCT TeamId, ApiKeyHash, TraceId
    FROM otel_traces
    WHERE {profile:String} != ''
      AND ({all_teams:UInt8}=1 OR TeamId={team:String})
      AND ({key_hash:String}='' OR ApiKeyHash={key_hash:String})
      AND TraceId={trace_id:String}
      AND hex(SHA256(concat(TeamId,char(0),ApiKeyHash,char(0),TraceId)))={trace_ref:String}
),
