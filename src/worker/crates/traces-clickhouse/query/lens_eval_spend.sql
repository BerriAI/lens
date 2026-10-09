WITH eval_traces AS (
    SELECT TeamId, ApiKeyHash, TraceId FROM lens_eval_traces
    WHERE ({all_teams:UInt8}=1 OR TeamId={team:String})
      AND ({key_hash:String}='' OR ApiKeyHash={key_hash:String})
), eval_call_keys AS (
    SELECT TeamId, ApiKeyHash,
        arrayJoin(if(empty(CallKeys),
            if(LiteLLMRequestId='', [], [concat('provider_response:', LiteLLMRequestId)]),
            CallKeys)) AS call_key
    FROM otel_traces
    WHERE (TeamId,ApiKeyHash,TraceId) IN eval_traces
), eval_transport AS (
    SELECT TeamId, ApiKeyHash,
        coalesce(nullIf(SpanAttributes['lens.original_trace_id'],''), TraceId) AS transport_trace,
        SpanId
    FROM otel_traces
    WHERE (TeamId,ApiKeyHash,TraceId) IN eval_traces
      AND hasAny(CallKeys, ['transport:', 'gateway_attempt:'])
),
(team, key, response, provider_request, gateway_call, request, trace, span) -> arrayExists(
    call_key -> (team, key, call_key) IN eval_call_keys,
    [concat('provider_response:', response),
     concat('provider_response:', if(startsWith(response, 'resp_'),
         extract(tryBase64Decode(substring(response, 6)), 'response_id:([^;]+)'), '')),
     concat('provider_request:', provider_request),
     concat('litellm_request:', if(gateway_call='', request, gateway_call))]
) OR (team, key, trace, span) IN eval_transport AS is_eval_request,
