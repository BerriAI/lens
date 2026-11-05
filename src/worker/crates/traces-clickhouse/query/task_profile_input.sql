SELECT substringUTF8(InputPreview, 1, 128) AS input,
       if(ObservationType='tool', substringUTF8(SpanName, 1, 64), '') AS tool,
       coalesce(nullIf(trimBoth(SpanAttributes['task_category']), ''),
           nullIf(trimBoth(ResourceAttributes['task_category']), ''),
           nullIf(trimBoth(SpanAttributes['scenario']), ''),
           nullIf(trimBoth(ResourceAttributes['scenario']), ''), '') AS recorded
FROM otel_traces
PREWHERE TeamId={team_id:String} AND ApiKeyHash={api_key_hash:String} AND TraceId={trace_id:String}
ORDER BY tuple(if(ParentSpanId='',0,1), Timestamp, SpanId)
LIMIT 8
