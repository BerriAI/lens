WITH greatest(toInt64({offset:UInt32})-1,1) AS content_offset,
(value, budget) -> if(lengthUTF8(value) <= budget, value,
    concat(substringUTF8(value, 1, intDiv(budget, 3)), '\n[... content omitted ...]\n',
        substringUTF8(value, -(budget - intDiv(budget, 3))))) AS excerpt
SELECT * FROM (
    SELECT SpanId AS span_id, ParentSpanId AS parent_span_id, SpanName AS name,
        ObservationType AS kind,
        InputTokens AS input_tokens, OutputTokens AS output_tokens,
        mapFilter((key, value) -> key IN (
            'gen_ai.usage.input_tokens', 'gen_ai.usage.prompt_tokens', 'gen_ai.usage.output_tokens',
            'gen_ai.usage.completion_tokens', 'llm.token_count.prompt', 'llm.token_count.completion',
            'ai.usage.promptTokens', 'ai.usage.completionTokens', 'ai.usage.tokens',
            'input_tokens', 'output_tokens', 'cache_read_tokens', 'cache_creation_tokens',
            'gen_ai.usage.cache_read.input_tokens', 'gen_ai.usage.cache_creation.input_tokens',
            'gen_ai.usage.input_tokens_details.cache_read', 'gen_ai.usage.input_tokens_details.cache_write'
        ), SpanAttributes) AS metric_attributes,
        trimBoth(Input) NOT IN ('', '[]', '{}', 'null') AS input_present,
        trimBoth(Output) NOT IN ('', '[]', '{}', 'null') AS output_present,
        CAST(NULL, 'Nullable(Float64)') AS gateway_cost_usd,
        toString(Timestamp, 'UTC') AS start_time,
        toString(addNanoseconds(Timestamp, Duration), 'UTC') AS end_time,
        multiIf({offset:UInt32}=0, concat('Input: ',Input,'\nOutput: ',Output,'\nStatus: ',StatusCode,' ',StatusMessage),
            {offset:UInt32}=1 AND lengthUTF8(concat('Input: ',Input,'\nOutput: ',Output,'\nStatus: ',StatusCode,' ',StatusMessage))>8000,
            concat('Input: ',excerpt(Input,2000),'\nOutput: ',excerpt(Output,5000),
                '\nStatus: ',StatusCode,' ',excerpt(StatusMessage,500)),
            substringUTF8(concat('Input: ',Input,'\nOutput: ',Output,'\nStatus: ',StatusCode,' ',StatusMessage),
                content_offset,8000)) AS content,
        {offset:UInt32}!=0 AND lengthUTF8(concat('Input: ',Input,'\nOutput: ',Output,'\nStatus: ',StatusCode,' ',StatusMessage))
            >= content_offset+8000 AS truncated
    FROM otel_traces WHERE {source:String}='traces'
      AND ({all_teams:UInt8}=1 OR TeamId={team:String})
      AND ({key_hash:String}='' OR ApiKeyHash={key_hash:String})
      AND Timestamp >= parseDateTime64BestEffortOrZero({start_time:String}, 9) - INTERVAL 7 DAY
      AND ({trace_ref:String}='' OR hex(SHA256(concat(TeamId, char(0), ApiKeyHash, char(0), TraceId)))={trace_ref:String})
      AND TraceId={id:String} AND TeamId={record_team:String} AND SpanId > {cursor:String}
      AND (TeamId, ApiKeyHash, TraceId) NOT IN (SELECT TeamId, ApiKeyHash, TraceId FROM lens_eval_traces
          WHERE TeamId={record_team:String} AND TraceId={id:String})
    ORDER BY SpanId LIMIT 1 BY SpanId LIMIT 40
)
UNION ALL
SELECT * FROM (
    SELECT request_id AS span_id, '' AS parent_span_id, model AS name, 'llm' AS kind,
        prompt_tokens AS input_tokens, completion_tokens AS output_tokens,
        mapFilter((key, value) -> value != '', map(
            'cache_read_tokens', if(cache_read_tokens > 0, toString(cache_read_tokens), ''),
            'cache_creation_tokens', if(cache_write_tokens > 0, toString(cache_write_tokens), '')
        )) AS metric_attributes,
        trimBoth(messages) NOT IN ('', '[]', '{}', 'null') AS input_present,
        trimBoth(response) NOT IN ('', '[]', '{}', 'null') AS output_present,
        spend AS gateway_cost_usd,
        toString(start_time, 'UTC') AS start_time,
        toString(end_time, 'UTC') AS end_time,
        multiIf({offset:UInt32}=0, concat('Input: ',messages,'\nOutput: ',response,'\nError: ',error_str),
            {offset:UInt32}=1 AND lengthUTF8(concat('Input: ',messages,'\nOutput: ',response,'\nError: ',error_str))>8000,
            concat('Input: ',excerpt(messages,2000),'\nOutput: ',excerpt(response,5000),'\nError: ',excerpt(error_str,500)),
            substringUTF8(concat('Input: ',messages,'\nOutput: ',response,'\nError: ',error_str),
                content_offset,8000)) AS content,
        {offset:UInt32}!=0 AND lengthUTF8(concat('Input: ',messages,'\nOutput: ',response,'\nError: ',error_str))
            >= content_offset+8000 AS truncated
    FROM spend_logs FINAL WHERE {source:String}='requests'
      AND ({all_teams:UInt8}=1 OR team_id={team:String})
      AND ({key_hash:String}='' OR api_key={key_hash:String})
      AND spend_logs.start_time >= parseDateTime64BestEffortOrZero({start_time:String}, 3) - INTERVAL 7 DAY
      AND request_id={id:String} AND team_id={record_team:String}
      AND (team_id, api_key, response_id) NOT IN (SELECT TeamId, ApiKeyHash, arrayJoin(RequestIds)
          FROM lens_eval_traces WHERE TeamId={record_team:String})
    LIMIT 1
)
