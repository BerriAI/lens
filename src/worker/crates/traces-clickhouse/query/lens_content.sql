WITH greatest(toInt64({offset:UInt32})-1,1) AS content_offset,
(value, budget) -> if(lengthUTF8(value) <= budget, value,
    concat(substringUTF8(value, 1, intDiv(budget, 3)), '\n[... content omitted ...]\n',
        substringUTF8(value, -(budget - intDiv(budget, 3))))) AS excerpt
SELECT * FROM (
    SELECT SpanId AS span_id, ParentSpanId AS parent_span_id, SpanName AS name,
        ObservationType AS kind,
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
