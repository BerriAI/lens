SELECT *, hex(sipHash128(
    {signal_config:String}, {signal_threshold:Float64},
    count() OVER (), groupBitXor(cityHash64(trace_id, trace_ref)) OVER ()
)) AS signal_snapshot
FROM sample_candidates
WHERE (trace_id, trace_ref) IN (
    WITH signal_keys AS (
        SELECT concat('trace-signal/', encodeURLComponent(
            replaceAll(toJSONString(tuple(trace_id, trace_ref)), concat(char(92), '/'), '/'))) AS key
        FROM sample_candidates
    )
    SELECT JSONExtractString(b.data, 'trace_id'), JSONExtractString(b.data, 'trace_ref')
    FROM lens_state_blobs AS b
    WHERE (b.key, b.revision, toString(b.digest)) IN (
          SELECT key, revision, digest FROM lens_state_heads
          WHERE key IN (SELECT key FROM signal_keys)
    )
      AND JSONExtractString(b.data, 'config_key') = {signal_config:String}
      AND JSONExtractString(b.data, 'data', 'status') = 'classified'
      AND JSONExtractString(b.data, 'data', 'error') = ''
      AND JSONHas(b.data, 'data', 'scores', {signal_id:String})
      AND JSONExtractFloat(b.data, 'data', 'scores', {signal_id:String}) BETWEEN {signal_threshold:Float64} AND 1
)
