SELECT sum(matches) AS count FROM (
    SELECT count() AS matches FROM otel_traces WHERE {source:String}='traces'
      AND ({all_teams:UInt8}=1 OR TeamId={team:String})
      AND ({key_hash:String}='' OR ApiKeyHash={key_hash:String})
      AND Timestamp >= parseDateTime64BestEffortOrZero({start_time:String}, 9) - INTERVAL 7 DAY
      AND ({trace_ref:String}='' OR hex(SHA256(concat(TeamId, char(0), ApiKeyHash, char(0), TraceId)))={trace_ref:String})
      AND TraceId={id:String} AND TeamId={record_team:String} AND SpanId={span:String}
      AND (TeamId, ApiKeyHash, TraceId) NOT IN (SELECT TeamId, ApiKeyHash, TraceId FROM lens_eval_traces
          WHERE TeamId={record_team:String} AND TraceId={id:String})
      AND position(concat('Input: ',Input,'\nOutput: ',Output,'\nStatus: ',StatusCode,' ',StatusMessage),{quote:String})>0
)
