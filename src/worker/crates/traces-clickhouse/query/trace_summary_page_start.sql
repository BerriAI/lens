page AS (
    SELECT hex(SHA256(concat(TeamId, char(0), ApiKeyHash, char(0), TraceId))) AS trace_ref,
           toUnixTimestamp64Milli(min(StartTs)) AS start_ms
    FROM agent_traces_by_key
    WHERE ({all_teams:UInt8} = 1
           OR ({user_id:String} != '' AND UserIds = [{user_id:String}])
           OR has({team_ids:Array(String)}, TeamId))
