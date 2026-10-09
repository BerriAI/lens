CREATE MATERIALIZED VIEW IF NOT EXISTS {database}.lens_eval_traces_mv
TO {database}.lens_eval_traces AS
SELECT TeamId, ApiKeyHash, TraceId,
       min(Timestamp)                                       AS StartTs,
       groupUniqArray(UserId)                               AS UserIds,
       groupUniqArrayIf(LiteLLMRequestId, LiteLLMRequestId != '') AS RequestIds
FROM {database}.otel_traces
WHERE has([ResourceAttributes['deployment.environment'], ResourceAttributes['deployment.environment.name'],
           SpanAttributes['deployment.environment'], SpanAttributes['deployment.environment.name']], 'lens-eval')
GROUP BY TeamId, ApiKeyHash, TraceId
