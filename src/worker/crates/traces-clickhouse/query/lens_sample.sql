concat(leftPad(toString(cityHash64(concat(source,team_id,trace_ref,trace_id))),20,'0'),
    hex(concat(source,char(0),team_id,char(0),trace_ref,char(0),trace_id))) AS selection_key,
sample_candidates AS (
    SELECT * FROM (
    SELECT 'traces' AS source, TraceId AS trace_id, TeamId AS team_id, hex(SHA256(concat(TeamId, char(0), ApiKeyHash, char(0), TraceId))) AS trace_ref,
        coalesce(nullIf(argMin(ResourceAttributes['run.name'], Timestamp), ''),
            argMin(SpanName, Timestamp)) AS name, concat(replaceOne(toString(toDateTime64(min(Timestamp), 3, 'UTC')), ' ', 'T'), 'Z') AS start_time,
        uniqExact(SpanId) AS span_count, countIf(ParentSpanId='') > 0 AS root_seen,
        argMin(ServiceName, Timestamp) AS service,
        arrayZip(mapKeys(argMin(mapConcat(ResourceAttributes, SpanAttributes), tuple(ParentSpanId!='',Timestamp))),
            mapValues(argMin(mapConcat(ResourceAttributes, SpanAttributes), tuple(ParentSpanId!='',Timestamp)))) AS attributes
    FROM otel_traces
    WHERE {source:String} IN ('traces','both')
      AND ({all_teams:UInt8}=1 OR TeamId={team:String})
      AND ({key_hash:String}='' OR ApiKeyHash={key_hash:String})
      AND (TeamId, ApiKeyHash, TraceId) NOT IN (SELECT TeamId, ApiKeyHash, TraceId FROM lens_eval_traces
          WHERE ({all_teams:UInt8}=1 OR TeamId={team:String})
            AND ({key_hash:String}='' OR ApiKeyHash={key_hash:String}))
      -- The 7 day slack covers spans that started before the window and late ingestion
      AND Timestamp >= fromUnixTimestamp64Milli(toInt64({start:UInt64})) - INTERVAL 7 DAY
      AND (TeamId,ApiKeyHash,TraceId) IN (
          SELECT TeamId,ApiKeyHash,TraceId FROM otel_traces
          WHERE ({all_teams:UInt8}=1 OR TeamId={team:String})
            AND ({key_hash:String}='' OR ApiKeyHash={key_hash:String})
            AND Timestamp >= fromUnixTimestamp64Milli(toInt64({start:UInt64})) - INTERVAL 7 DAY
            AND Timestamp < fromUnixTimestamp64Milli(toInt64({end:UInt64}))
            AND if(EngineReceivedMs>0,toInt64(EngineReceivedMs),
                toUnixTimestamp64Milli(Timestamp)+toInt64(intDiv(Duration,1000000))) >= {start:UInt64}
      )
      AND ({profile:String}='' OR (TeamId,ApiKeyHash,TraceId) IN (
          SELECT TeamId,ApiKeyHash,TraceId FROM trace_profiles
          WHERE if(match({profile:String}, '^(recorded|task):') OR {profile:String} IN ('unconfigured', 'unclassified', 'pending', 'unknown'),
                   profile_id, category) = {profile:String}
               OR ({profile:String} IN ('unclassified', 'Unclassified') AND profile_id = 'unconfigured')
      ))
    GROUP BY TeamId,ApiKeyHash,TraceId
    HAVING max(EngineReceivedMs) < {end:UInt64}
       AND max(toUnixTimestamp64Milli(Timestamp)+toInt64(intDiv(Duration,1000000))) < {end:UInt64}
       AND ({agent_name:String}='' OR countIf(AgentName={agent_name:String}) > 0)
       AND countIf(AgentName='lens-signals') = 0
       AND countIf((empty({filter_keys:Array(String)}) OR arrayAll((k,v) -> ResourceAttributes[k]=v OR SpanAttributes[k]=v,
           {filter_keys:Array(String)},{filter_values:Array(String)}))
           AND ({service:String}='' OR ServiceName={service:String})) > 0
)
WHERE ({selected_team:String}='' OR team_id={selected_team:String})
  AND (empty({execution_ids:Array(String)}) OR has({execution_ids:Array(String)},
      concat(source,char(0),team_id,char(0),if(trace_ref='',trace_id,trace_ref))))
),
