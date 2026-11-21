SELECT team_id, api_key, request_id, error_str,
    JSONExtractRaw(metadata, 'litellm_lens_content') AS reference
FROM spend_logs FINAL
WHERE ({all_teams:UInt8}=1 OR team_id={team:String})
  AND ({key_hash:String}='' OR api_key={key_hash:String})
  AND spend_logs.start_time >= parseDateTime64BestEffortOrZero({start_time:String}, 3) - INTERVAL 7 DAY
  AND request_id={id:String} AND team_id={record_team:String}
  AND messages='' AND response=''
  AND JSONHas(metadata, 'litellm_lens_content')
  AND (team_id, api_key, response_id) NOT IN (SELECT TeamId, ApiKeyHash, arrayJoin(RequestIds)
      FROM lens_eval_traces WHERE TeamId={record_team:String})
LIMIT 1
