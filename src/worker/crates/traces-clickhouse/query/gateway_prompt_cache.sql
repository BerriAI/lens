SELECT sumIf(prompt_tokens, measured) AS input_tokens,
       sumIf(cache_read_tokens, measured) AS read_tokens,
       sumIf(cache_write_tokens, measured) AS write_tokens,
       if(uniqExactIf(requests.session_id, requests.session_id != '') = 1,
          anyIf(requests.session_id, requests.session_id != '' AND requests.session_id != requests.trace_id), '') AS session_id
FROM (
    SELECT prompt_tokens, cache_read_tokens, cache_write_tokens, session_id, trace_id,
           (NOT cache_hit
            AND prompt_tokens > 0
            AND toUInt64(cache_read_tokens) + toUInt64(cache_write_tokens) <= prompt_tokens
            AND (cache_read_tokens > 0 OR cache_write_tokens > 0
              OR JSONExtractRaw(metadata, 'usage_object', 'prompt_tokens_details', 'cached_tokens') NOT IN ('', 'null')
              OR JSONExtractRaw(metadata, 'usage_object', 'prompt_tokens_details', 'cache_write_tokens') NOT IN ('', 'null')
              OR JSONExtractRaw(metadata, 'usage_object', 'prompt_tokens_details', 'cache_creation_tokens') NOT IN ('', 'null')
              OR JSONExtractRaw(metadata, 'usage_object', 'cache_read_input_tokens') NOT IN ('', 'null')
              OR JSONExtractRaw(metadata, 'usage_object', 'cache_creation_input_tokens') NOT IN ('', 'null'))) AS measured
    FROM spend_logs FINAL
    WHERE team_id = {team:String}
      AND if({owner_key:String} != '', api_key = {owner_key:String},
             {owner_user:String} != '' AND user = {owner_user:String})
      AND ({all_teams:UInt8} = 1 OR has({team_ids:Array(String)}, team_id)
           OR ({user_id:String} != '' AND user = {user_id:String}))
      AND EngineReceivedMs <= {snapshot_ms:UInt64}
      AND if({session:String} != '', session_id = {session:String},
             trace_id IN {trace_ids:Array(String)}
             OR (request_id IN {request_ids:Array(String)}
                 AND start_time >= fromUnixTimestamp64Milli({start_ms:Int64})
                 AND start_time < fromUnixTimestamp64Milli({end_ms:Int64})))
) AS requests
