(span_attributes, resource_attributes) -> coalesce(
    nullIf(trimBoth(span_attributes['task_category']), ''),
    nullIf(trimBoth(resource_attributes['task_category']), ''),
    nullIf(trimBoth(span_attributes['scenario']), ''),
    nullIf(trimBoth(resource_attributes['scenario']), ''), '') AS recorded_profile,
trace_profiles AS (
    SELECT TeamId, ApiKeyHash, TraceId,
           if(selected_profile='', 'Unclassified', selected_profile) AS category
    FROM (
        SELECT TeamId, ApiKeyHash, TraceId,
               argMinIf(recorded_profile(SpanAttributes,ResourceAttributes),
                        tuple(if(ParentSpanId='',0,1),Timestamp,SpanId),
                        recorded_profile(SpanAttributes,ResourceAttributes)!='') AS selected_profile
        FROM otel_traces
        WHERE (TeamId,ApiKeyHash,TraceId) IN (SELECT TeamId,ApiKeyHash,TraceId FROM profile_candidates)
        GROUP BY TeamId,ApiKeyHash,TraceId
    )
),
