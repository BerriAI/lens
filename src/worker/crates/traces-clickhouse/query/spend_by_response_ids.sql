WITH ledger_candidates AS (
    SELECT DISTINCT toDate(event_time) AS event_day, team_id, event_id
    FROM lens_ledger_aliases
    WHERE event_time >= fromUnixTimestamp64Milli({start_ms:Int64})
      AND event_time < fromUnixTimestamp64Milli({end_ms:Int64})
      AND ({all_teams:UInt8} = 1
           OR ({user_id:String} != '' AND user = {user_id:String})
           OR has({team_ids:Array(String)}, team_id))
      AND ((key_kind = 'provider_request' AND key_value IN {provider_request_ids:Array(String)})
        OR (key_kind = 'provider_response' AND key_value IN {response_ids:Array(String)})
        OR (key_kind = 'litellm_request' AND key_value IN {request_ids:Array(String)})
        OR (key_kind = 'transport' AND key_value IN {trace_ids:Array(String)}))
), ledger_matched AS (
    SELECT DISTINCT team_id, api_key, user, litellm_call_id
    FROM lens_ledger_costs FINAL
    WHERE (toDate(event_time), team_id, event_id) IN (SELECT event_day, team_id, event_id FROM ledger_candidates)
      AND event_time >= fromUnixTimestamp64Milli({start_ms:Int64})
      AND event_time < fromUnixTimestamp64Milli({end_ms:Int64})
      AND ({all_teams:UInt8} = 1
           OR ({user_id:String} != '' AND user = {user_id:String})
           OR has({team_ids:Array(String)}, team_id))
      AND (provider_request_id IN {provider_request_ids:Array(String)}
        OR response_id IN {response_ids:Array(String)}
        OR upstream_response_id IN {response_ids:Array(String)}
        OR litellm_call_id IN {request_ids:Array(String)}
        OR (litellm_call_id = '' AND request_id IN {request_ids:Array(String)})
        OR (trace_id != '' AND trace_id IN {trace_ids:Array(String)}))
      AND litellm_call_id != ''
), ledger_siblings AS (
    SELECT DISTINCT toDate(event_time) AS event_day, team_id, event_id
    FROM lens_ledger_aliases
    WHERE key_kind = 'litellm_request'
      AND key_value IN (SELECT litellm_call_id FROM ledger_matched)
      AND (team_id, api_key, user, key_value) IN (SELECT team_id, api_key, user, litellm_call_id FROM ledger_matched)
      AND event_time >= fromUnixTimestamp64Milli({start_ms:Int64})
      AND event_time < fromUnixTimestamp64Milli({end_ms:Int64})
      AND ({all_teams:UInt8} = 1
           OR ({user_id:String} != '' AND user = {user_id:String})
           OR has({team_ids:Array(String)}, team_id))
), ledger AS (
    SELECT request_id, event_id, litellm_call_id, response_id, upstream_response_id, provider_request_id,
           trace_id, span_id, team_id, api_key, user, spend,
           toUnixTimestamp64Milli(event_time) AS start_ms
    FROM lens_ledger_costs FINAL
    WHERE (toDate(event_time), team_id, event_id) IN (
              SELECT event_day, team_id, event_id FROM ledger_candidates
              UNION ALL
              SELECT event_day, team_id, event_id FROM ledger_siblings)
      AND event_time >= fromUnixTimestamp64Milli({start_ms:Int64})
      AND event_time < fromUnixTimestamp64Milli({end_ms:Int64})
      AND ({all_teams:UInt8} = 1
           OR ({user_id:String} != '' AND user = {user_id:String})
           OR has({team_ids:Array(String)}, team_id))
      AND ((provider_request_id IN {provider_request_ids:Array(String)}
        OR response_id IN {response_ids:Array(String)}
        OR upstream_response_id IN {response_ids:Array(String)}
        OR litellm_call_id IN {request_ids:Array(String)}
        OR (litellm_call_id = '' AND request_id IN {request_ids:Array(String)})
        OR (trace_id != '' AND trace_id IN {trace_ids:Array(String)}))
        OR (litellm_call_id != ''
            AND (team_id, api_key, user, litellm_call_id) IN (SELECT team_id, api_key, user, litellm_call_id FROM ledger_matched)))
), legacy_candidates AS (
    SELECT DISTINCT request_id, team_id, start_time
    FROM lens_call_costs
    WHERE start_time >= fromUnixTimestamp64Milli({start_ms:Int64})
      AND start_time < fromUnixTimestamp64Milli({end_ms:Int64})
      AND ({all_teams:UInt8} = 1
           OR ({user_id:String} != '' AND user = {user_id:String})
           OR has({team_ids:Array(String)}, team_id))
      AND ((key_kind = 'provider_request' AND key_value IN {provider_request_ids:Array(String)})
        OR (key_kind = 'provider_response' AND key_value IN {response_ids:Array(String)})
        OR (key_kind = 'litellm_request' AND key_value IN {request_ids:Array(String)})
        OR (key_kind = 'transport' AND key_value IN {trace_ids:Array(String)}))
), legacy AS (
    SELECT request_id, '' AS event_id, litellm_call_id, response_id, upstream_response_id, provider_request_id,
           trace_id, span_id, team_id, api_key, user, spend,
           toUnixTimestamp64Milli(start_time) AS start_ms
    FROM lens_call_costs FINAL
    WHERE key_kind = 'canonical'
      AND (key_value, team_id, start_time) IN (SELECT request_id, team_id, start_time FROM legacy_candidates)
      AND start_time >= fromUnixTimestamp64Milli({start_ms:Int64})
      AND start_time < fromUnixTimestamp64Milli({end_ms:Int64})
      AND ({all_teams:UInt8} = 1
           OR ({user_id:String} != '' AND user = {user_id:String})
           OR has({team_ids:Array(String)}, team_id))
      AND (provider_request_id IN {provider_request_ids:Array(String)}
        OR response_id IN {response_ids:Array(String)}
        OR upstream_response_id IN {response_ids:Array(String)}
        OR litellm_call_id IN {request_ids:Array(String)}
        OR (litellm_call_id = '' AND request_id IN {request_ids:Array(String)})
        OR (trace_id != '' AND trace_id IN {trace_ids:Array(String)}))
), ledger_calls AS (
    SELECT DISTINCT team_id, api_key, litellm_call_id
    FROM lens_ledger_costs
    WHERE litellm_call_id != ''
      AND event_time >= fromUnixTimestamp64Milli({start_ms:Int64}) - INTERVAL 1 DAY
      AND event_time < fromUnixTimestamp64Milli({end_ms:Int64}) + INTERVAL 1 DAY
      AND team_id IN (SELECT team_id FROM legacy WHERE litellm_call_id != '')
      AND (team_id, api_key, litellm_call_id) IN (
          SELECT team_id, api_key, litellm_call_id FROM legacy WHERE litellm_call_id != '')
)
SELECT * FROM (
    SELECT * FROM ledger
    UNION ALL
    SELECT * FROM legacy
    WHERE litellm_call_id = ''
       OR (team_id, api_key, litellm_call_id) NOT IN (SELECT team_id, api_key, litellm_call_id FROM ledger_calls)
)
ORDER BY start_ms DESC
