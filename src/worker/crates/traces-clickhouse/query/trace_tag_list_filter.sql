AND (empty({filter_keys:Array(String)}) OR (TeamId, ApiKeyHash, TraceId) IN (
      SELECT TeamId, ApiKeyHash, TraceId FROM otel_traces
      LEFT JOIN (SELECT TeamId AS ManualTeamId, ApiKeyHash AS ManualKeyHash, TraceId AS ManualTraceId,
                               Tags AS ManualTags FROM lens_trace_tags FINAL) AS manual
      ON TeamId=ManualTeamId AND ApiKeyHash=ManualKeyHash AND TraceId=ManualTraceId
      LEFT JOIN trace_request_tags USING (TeamId,ApiKeyHash,TraceId)
      WHERE Timestamp >= fromUnixTimestamp64Milli({start_ms:Int64})
        AND arrayAll((k, v) -> matches_trace_tag(k,v,ResourceAttributes,SpanAttributes,ManualTags,RequestTags),
                     {filter_keys:Array(String)}, {filter_values:Array(String)})
  ))
