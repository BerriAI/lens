SELECT team_id, request_id, toUnixTimestamp64Milli(start_time) AS start_ms,
       model_id, model_group, model, toString(custom_llm_provider) AS provider
FROM lens_call_costs FINAL
WHERE key_kind = 'canonical'
  AND key_value IN {request_ids:Array(String)}
  AND start_time >= fromUnixTimestamp64Milli({start_ms:Int64})
  AND start_time < fromUnixTimestamp64Milli({end_ms:Int64})
  AND ({all_teams:UInt8} = 1
       OR ({user_id:String} != '' AND user = {user_id:String})
       OR has({team_ids:Array(String)}, team_id))
