page AS (
    SELECT hex(SHA256(concat(TeamId, char(0), ApiKeyHash, char(0), TraceId))) AS trace_ref,
           toUnixTimestamp64Milli(min(StartTs)) AS start_ms
    FROM agent_traces_by_key
    WHERE ({all_teams:UInt8} = 1
           OR ({user_id:String} != '' AND UserIds = [{user_id:String}])
           OR has({team_ids:Array(String)}, TeamId))
      AND (TeamId, ApiKeyHash, TraceId) NOT IN (
          SELECT TeamId, ApiKeyHash, TraceId FROM lens_eval_traces
          WHERE ({all_teams:UInt8} = 1
                 OR ({user_id:String} != '' AND has(UserIds, {user_id:String}))
                 OR has({team_ids:Array(String)}, TeamId))
          GROUP BY TeamId, ApiKeyHash, TraceId
          HAVING min(StartTs) >= fromUnixTimestamp64Milli({start_ms:Int64}))
    GROUP BY TeamId, ApiKeyHash, TraceId
    HAVING min(StartTs) >= fromUnixTimestamp64Milli({start_ms:Int64})
       AND min(StartTs) < fromUnixTimestamp64Milli({end_ms:Int64})
       AND ({cursor_ms:Int64} = 0 OR (toUnixTimestamp64Milli(min(StartTs)), trace_ref)
            < ({cursor_ms:Int64}, {cursor_trace_id:String}))
    ORDER BY start_ms DESC, trace_ref DESC
    LIMIT {limit:UInt32}
), selected_refs AS (
    SELECT trace_ref FROM page
),
