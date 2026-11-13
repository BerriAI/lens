(span_attributes, resource_attributes) -> coalesce(
    nullIf(trimBoth(span_attributes['task_category']), ''),
    nullIf(trimBoth(resource_attributes['task_category']), ''),
    nullIf(trimBoth(span_attributes['scenario']), ''),
    nullIf(trimBoth(resource_attributes['scenario']), ''), '') AS recorded_profile,
trace_profiles AS (
    SELECT TeamId, ApiKeyHash, TraceId, recorded, cluster_id, cluster_label, task_id, outcome,
           multiIf(recorded != '', concat('recorded:', recorded),
                   {taxonomy_version:UInt32} > 0 AND outcome = 'assigned' AND task_id != '', concat('task:', task_id),
                   {taxonomy_version:UInt32} > 0 AND outcome = 'unknown', 'unknown',
                   {taxonomy_version:UInt32} > 0
                       AND arrayExists((s, e) -> run_start_ms >= s AND run_start_ms < e,
                                       {window_starts:Array(Int64)}, {window_ends:Array(Int64)}), 'pending',
                   cluster_id != '', concat('cluster:', cluster_id),
                   cluster_label != '', 'uncertain',
                   'unclassified') AS profile_id,
           multiIf(recorded != '', recorded,
                   {taxonomy_version:UInt32} > 0 AND outcome = 'assigned' AND task_id != '', '',
                   {taxonomy_version:UInt32} > 0 AND outcome = 'unknown', 'Unknown',
                   {taxonomy_version:UInt32} > 0
                       AND arrayExists((s, e) -> run_start_ms >= s AND run_start_ms < e,
                                       {window_starts:Array(Int64)}, {window_ends:Array(Int64)}), 'Pending classification',
                   cluster_label != '', cluster_label,
                   'Unclassified') AS category
    FROM (
        SELECT TeamId, ApiKeyHash, TraceId,
               argMinIf(recorded_profile(SpanAttributes,ResourceAttributes),
                        tuple(if(ParentSpanId='',0,1),Timestamp,SpanId),
                        recorded_profile(SpanAttributes,ResourceAttributes)!='') AS recorded,
               toUnixTimestamp64Milli(min(Timestamp)) AS run_start_ms
        FROM otel_traces
        WHERE (TeamId,ApiKeyHash,TraceId) IN (SELECT TeamId,ApiKeyHash,TraceId FROM profile_candidates)
        GROUP BY TeamId,ApiKeyHash,TraceId
    ) AS recorded_rows
    LEFT JOIN (
        SELECT TeamId, ApiKeyHash, TraceId,
               argMax(ClusterId, ComputedAt) AS cluster_id, argMax(Category, ComputedAt) AS cluster_label
        FROM lens_task_profiles
        WHERE ResolutionVersion=1
          AND (TeamId,ApiKeyHash,TraceId) IN (SELECT TeamId,ApiKeyHash,TraceId FROM profile_candidates)
        GROUP BY TeamId,ApiKeyHash,TraceId
    ) AS inferred USING (TeamId,ApiKeyHash,TraceId)
    LEFT JOIN (
        SELECT TeamId, ApiKeyHash, TraceId,
               argMax(TaskId, ComputedAt) AS task_id, argMax(Outcome, ComputedAt) AS outcome
        FROM lens_task_profile_assignments
        WHERE TaxonomyId={taxonomy_id:String} AND TaxonomyVersion={taxonomy_version:UInt32}
          AND (TeamId,ApiKeyHash,TraceId) IN (SELECT TeamId,ApiKeyHash,TraceId FROM profile_candidates)
        GROUP BY TeamId,ApiKeyHash,TraceId
    ) AS enhanced USING (TeamId,ApiKeyHash,TraceId)
),
