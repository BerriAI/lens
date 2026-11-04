SELECT TeamId AS team_id, concat(replaceOne(toString(toDateTime64(min(Timestamp), 3, 'UTC')), ' ', 'T'), 'Z') AS start_time
FROM otel_traces
LEFT JOIN (SELECT TeamId AS ManualTeamId, ApiKeyHash AS ManualKeyHash, TraceId AS ManualTraceId,
                  Tags AS ManualTags FROM lens_trace_tags FINAL) AS manual
ON TeamId=ManualTeamId AND ApiKeyHash=ManualKeyHash AND TraceId=ManualTraceId
WHERE ({all_teams:UInt8}=1 OR TeamId={team:String})
  AND ({key_hash:String}='' OR ApiKeyHash={key_hash:String})
  AND ({selected_team:String}='' OR TeamId={selected_team:String})
  AND TraceId={trace_id:String}
  AND hex(SHA256(concat(TeamId, char(0), ApiKeyHash, char(0), TraceId)))={trace_ref:String}
  AND (TeamId, ApiKeyHash, TraceId) NOT IN (
      SELECT TeamId, ApiKeyHash, TraceId FROM lens_eval_traces WHERE TraceId={trace_id:String})
GROUP BY TeamId, ApiKeyHash, TraceId
HAVING countIf(AgentName={agent_name:String}) > 0
   AND countIf(arrayAll((k,v) -> ResourceAttributes[k]=v OR SpanAttributes[k]=v OR ManualTags[k]=v,
       {filter_keys:Array(String)}, {filter_values:Array(String)})
       AND ({service:String}='' OR ServiceName={service:String})) > 0
LIMIT 2
