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
), seed_calls AS (
    SELECT team_id, api_key, user, litellm_call_id FROM ledger_matched
    UNION ALL
    SELECT team_id, api_key, user, litellm_call_id FROM legacy WHERE litellm_call_id != ''
), ledger_siblings AS (
    SELECT DISTINCT toDate(event_time) AS event_day, team_id, event_id
    FROM lens_ledger_aliases
    WHERE key_kind = 'litellm_request'
      AND key_value IN (SELECT litellm_call_id FROM seed_calls)
      AND (team_id, api_key, user, key_value) IN (SELECT team_id, api_key, user, litellm_call_id FROM seed_calls)
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
            AND (team_id, api_key, user, litellm_call_id) IN (SELECT team_id, api_key, user, litellm_call_id FROM seed_calls)))
), ledger_attempts AS (
    SELECT team_id, api_key, user, litellm_call_id, request_id, response_id, provider_request_id,
           toUnixTimestamp64Milli(event_time) AS start_ms
    FROM lens_ledger_costs FINAL
    WHERE litellm_call_id != ''
      AND event_time >= fromUnixTimestamp64Milli({start_ms:Int64}) - INTERVAL 1 DAY
      AND event_time < fromUnixTimestamp64Milli({end_ms:Int64}) + INTERVAL 1 DAY
      AND ({all_teams:UInt8} = 1
           OR ({user_id:String} != '' AND user = {user_id:String})
           OR has({team_ids:Array(String)}, team_id))
      AND team_id IN (SELECT team_id FROM legacy WHERE litellm_call_id != '')
      AND (team_id, api_key, litellm_call_id) IN (
          SELECT team_id, api_key, litellm_call_id FROM legacy WHERE litellm_call_id != '')
), legacy_attempts AS (
    SELECT legacy.team_id AS attempt_team, legacy.start_ms AS attempt_start, legacy.request_id AS attempt_request,
           max(attempt.litellm_call_id != ''
               AND ((((legacy.response_id != '' AND attempt.response_id != '')
                      OR (legacy.provider_request_id != '' AND attempt.provider_request_id != ''))
                     AND NOT (legacy.response_id != '' AND attempt.response_id != ''
                              AND legacy.response_id != attempt.response_id)
                     AND NOT (legacy.provider_request_id != '' AND attempt.provider_request_id != ''
                              AND legacy.provider_request_id != attempt.provider_request_id))
                    OR (NOT (legacy.response_id != '' AND attempt.response_id != '')
                        AND NOT (legacy.provider_request_id != '' AND attempt.provider_request_id != '')
                        AND legacy.request_id = attempt.request_id
                        AND legacy.start_ms = attempt.start_ms))) AS shadowed,
           max(attempt.litellm_call_id != '' AND attempt.user = legacy.user) AS joins_ledger
    FROM legacy
    LEFT JOIN ledger_attempts AS attempt
        ON attempt.team_id = legacy.team_id
       AND attempt.api_key = legacy.api_key
       AND attempt.litellm_call_id = legacy.litellm_call_id
    GROUP BY legacy.team_id, legacy.start_ms, legacy.request_id
)
SELECT * FROM (
    SELECT *, litellm_call_id AS ledger_call_id FROM ledger
    UNION ALL
    SELECT legacy.request_id, legacy.event_id, legacy.litellm_call_id, legacy.response_id,
           legacy.upstream_response_id, legacy.provider_request_id, legacy.trace_id, legacy.span_id,
           legacy.team_id, legacy.api_key, legacy.user, legacy.spend, legacy.start_ms,
           if(attempts.joins_ledger = 1, legacy.litellm_call_id, '') AS ledger_call_id
    FROM legacy
    INNER JOIN legacy_attempts AS attempts
        ON attempts.attempt_team = legacy.team_id
       AND attempts.attempt_start = legacy.start_ms
       AND attempts.attempt_request = legacy.request_id
    WHERE attempts.shadowed = 0
)
ORDER BY start_ms DESC
