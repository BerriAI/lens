profile_candidates AS (
    SELECT DISTINCT TeamId, ApiKeyHash, TraceId
    FROM otel_traces
    WHERE {profile:String} != ''
      AND ({all_teams:UInt8}=1 OR TeamId={team:String})
      AND ({key_hash:String}='' OR ApiKeyHash={key_hash:String})
      AND Timestamp >= fromUnixTimestamp64Milli(toInt64({start:UInt64})) - INTERVAL 7 DAY
      AND Timestamp < fromUnixTimestamp64Milli(toInt64({end:UInt64}))
),
