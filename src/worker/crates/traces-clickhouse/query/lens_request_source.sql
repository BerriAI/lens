SELECT request_id AS span_id, model AS name,
    toString(start_time, 'UTC') AS start_time, toString(end_time, 'UTC') AS end_time,
    messages, response, error_str, ledger_event_id AS event_id, team_id, api_key
FROM spend_logs FINAL WHERE {source:String}='requests'
  AND ({all_teams:UInt8}=1 OR team_id={team:String})
  AND ({key_hash:String}='' OR api_key={key_hash:String})
  AND spend_logs.start_time >= parseDateTime64BestEffortOrZero({start_time:String}, 3) - INTERVAL 7 DAY
  AND request_id={id:String} AND team_id={record_team:String} AND ledger_event_id != ''
  AND (team_id, api_key, response_id) NOT IN (SELECT TeamId, ApiKeyHash, arrayJoin(RequestIds)
      FROM lens_eval_traces WHERE TeamId={record_team:String})
LIMIT 1
