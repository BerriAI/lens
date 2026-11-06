(attributes) -> arraySlice(arrayDistinct(arrayConcat(
    JSONExtract(attributes['request_tags'], 'Array(String)'),
    JSONExtract(attributes['metadata.tags'], 'Array(String)'),
    JSONExtract(attributes['tags'], 'Array(String)'),
    JSONExtract(attributes['litellm.request_tags'], 'Array(String)'),
    JSONExtract(attributes['metadata'], 'tags', 'Array(String)'),
    JSONExtract(attributes['metadata.requester_metadata'], 'tags', 'Array(String)')
)), 1, 64) AS direct_request_tags,
(key, value, resource, span, manual, upstream) ->
    resource[key]=value OR span[key]=value OR manual[key]=value
    OR (key='tag' AND has(upstream,value)) AS matches_trace_tag,
tag_call_keys AS (
    SELECT TeamId, ApiKeyHash, UserId, TraceId, SpanId, Timestamp, Duration,
           arrayJoin(arrayMap(key -> if(key IN ('transport:', 'gateway_attempt:'),
               concat('transport:', coalesce(nullIf(SpanAttributes['lens.original_trace_id'],''),TraceId)), key),
               if(empty(CallKeys), if(LiteLLMRequestId='', [], [concat('provider_response:',LiteLLMRequestId)]), CallKeys))) AS call_key
    FROM tag_spans
), tagged_calls AS (
    SELECT spans.TeamId AS TeamId, spans.ApiKeyHash AS ApiKeyHash, spans.TraceId AS TraceId,
           spans.SpanId AS SpanId, spans.call_key AS call_key,
           any(current.start_time) AS start_time, any(current.request_id) AS request_id,
           any(current.api_key) AS matched_key
    FROM tag_call_keys AS spans
    INNER JOIN lens_call_costs AS costs
      ON spans.TeamId=costs.team_id AND spans.call_key=concat(costs.key_kind,':',costs.key_value)
    INNER JOIN (SELECT * FROM lens_call_costs FINAL WHERE key_kind='canonical') AS current
      ON costs.team_id=current.team_id AND costs.start_time=current.start_time
     AND costs.request_id=current.request_id
    WHERE ((spans.UserId!='' AND current.user=spans.UserId)
           OR (spans.ApiKeyHash!='' AND current.api_key=spans.ApiKeyHash))
      AND costs.start_time >= spans.Timestamp - INTERVAL 30 MINUTE
      AND costs.start_time <= spans.Timestamp + toIntervalMillisecond(intDiv(spans.Duration,1000000)) + INTERVAL 30 MINUTE
      AND spans.call_key IN (
          concat('provider_response:',current.response_id),
          concat('provider_response:',current.upstream_response_id),
          concat('provider_request:',current.provider_request_id),
          concat('litellm_request:',if(current.litellm_call_id='',current.request_id,current.litellm_call_id)),
          concat('transport:',current.trace_id)
      )
      AND (costs.key_kind!='transport' OR current.span_id=spans.SpanId)
    GROUP BY spans.TeamId, spans.ApiKeyHash, spans.TraceId, spans.SpanId, spans.call_key
    HAVING uniqExact(tuple(current.api_key,current.start_time,current.request_id))=1
), tag_sources AS (
    SELECT TeamId, ApiKeyHash, TraceId,
           arrayConcat(direct_request_tags(ResourceAttributes), direct_request_tags(SpanAttributes)) AS values
    FROM tag_spans
    UNION ALL
    SELECT calls.TeamId AS TeamId, calls.ApiKeyHash AS ApiKeyHash, calls.TraceId AS TraceId,
           spend.request_tags AS values
    FROM tagged_calls AS calls
    INNER JOIN spend_logs AS spend FINAL
      ON calls.TeamId=spend.team_id AND calls.matched_key=spend.api_key
     AND calls.start_time=spend.start_time AND calls.request_id=spend.request_id
), trace_request_tags AS (
    SELECT TeamId, ApiKeyHash, TraceId,
           arraySort(groupUniqArrayArray(64)(arrayFilter(value ->
               length(value)<=256 AND notEmpty(trimBoth(value)), values))) AS RequestTags
    FROM tag_sources GROUP BY TeamId,ApiKeyHash,TraceId
),
