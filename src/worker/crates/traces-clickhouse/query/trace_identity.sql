SELECT hex(SHA256(concat(TeamId, char(0), ApiKeyHash, char(0), TraceId))) AS trace_ref
FROM otel_traces
WHERE TraceId = {trace_id:String}
  AND ({all_teams:UInt8} = 1
       OR ({user_id:String} != '' AND UserId = {user_id:String})
       OR has({team_ids:Array(String)}, TeamId))
  AND ({filter_team_id:String} = '' OR TeamId = {filter_team_id:String})
  AND ({filter_api_key_hash:String} = '' OR ApiKeyHash = {filter_api_key_hash:String})
  AND ({filter_api_key_hashes_mode:String} = 'all' OR has({filter_api_key_hashes:Array(String)}, ApiKeyHash))
GROUP BY TeamId, ApiKeyHash, TraceId
LIMIT 2
