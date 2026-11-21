WITH candidates AS (
    SELECT TeamId, ApiKeyHash, TraceId,
        hex(SHA256(concat(TeamId, char(0), ApiKeyHash, char(0), TraceId))) AS trace_ref,
        toUnixTimestamp64Milli(min(StartTs)) AS start_ms,
        toUInt64(greatest(0, toUnixTimestamp64Milli(max(EndTs)) - start_ms)) AS duration_ms
    FROM agent_traces_by_key
    WHERE (TeamId, ApiKeyHash, TraceId) NOT IN (SELECT TeamId, ApiKeyHash, TraceId FROM lens_eval_traces)
    GROUP BY TeamId, ApiKeyHash, TraceId
    HAVING start_ms >= {since_ms:Int64} AND start_ms < {now_ms:Int64}
        AND trace_ref > {after:String}
        AND (({duration_ms:UInt64} > 0 AND duration_ms > {duration_ms:UInt64})
            OR ({model_errors:Bool} AND sum(ErrorCount) > 0))
        AND NOT has(groupUniqArrayArray(AgentNames), 'lens-signals')
    ORDER BY trace_ref
    LIMIT {limit:UInt32}
)
SELECT c.TraceId AS trace_id, c.trace_ref AS trace_ref, c.start_ms AS start_ms,
    c.duration_ms AS duration_ms,
    argMax(o.SpanId, toUnixTimestamp64Nano(o.Timestamp) + toInt64(o.Duration)) AS duration_span_id,
    argMinIf(o.SpanId, o.Timestamp, o.ObservationType = 'llm' AND o.StatusCode = 'STATUS_CODE_ERROR') AS error_span_id
FROM candidates AS c
INNER JOIN otel_traces AS o USING (TeamId, ApiKeyHash, TraceId)
GROUP BY c.TraceId, c.trace_ref, c.start_ms, c.duration_ms
ORDER BY trace_ref
