SELECT TeamId AS team_id, concat(replaceOne(toString(toDateTime64(min(Timestamp), 3, 'UTC')), ' ', 'T'), 'Z') AS start_time
FROM otel_traces
LEFT JOIN (SELECT TeamId AS ManualTeamId, ApiKeyHash AS ManualKeyHash, TraceId AS ManualTraceId,
                  Tags AS ManualTags FROM lens_trace_tags FINAL) AS manual
ON TeamId=ManualTeamId AND ApiKeyHash=ManualKeyHash AND TraceId=ManualTraceId
LEFT JOIN trace_request_tags USING (TeamId,ApiKeyHash,TraceId)
WHERE ({all_teams:UInt8}=1 OR TeamId={team:String})
  AND ({key_hash:String}='' OR ApiKeyHash={key_hash:String})
  AND ({selected_team:String}='' OR TeamId={selected_team:String})
  AND TraceId={trace_id:String}
  AND hex(SHA256(concat(TeamId, char(0), ApiKeyHash, char(0), TraceId)))={trace_ref:String}
  AND (TeamId, ApiKeyHash, TraceId) NOT IN (
      SELECT TeamId, ApiKeyHash, TraceId FROM lens_eval_traces WHERE TraceId={trace_id:String})
  AND ({profile:String}='' OR (TeamId,ApiKeyHash,TraceId) IN (
      SELECT TeamId,ApiKeyHash,TraceId FROM trace_profiles WHERE category={profile:String}
  ))
GROUP BY TeamId, ApiKeyHash, TraceId
HAVING ({agent_name:String}='' OR countIf(AgentName={agent_name:String}) > 0)
   AND countIf(arrayAll((k,v) -> matches_trace_tag(k,v,ResourceAttributes,SpanAttributes,ManualTags,RequestTags),
       {filter_keys:Array(String)}, {filter_values:Array(String)})
       AND ({service:String}='' OR ServiceName={service:String})) > 0
LIMIT 2
