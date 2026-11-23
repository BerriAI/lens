SELECT * FROM (
    SELECT *, if({signal_sample_cap:UInt64}=0, ceiling(eligible*{signal_sample_percent:Float64}/100),
        least(toFloat64({signal_sample_cap:UInt64}),ceiling(eligible*{signal_sample_percent:Float64}/100))) AS selected
    FROM (
        SELECT *, count() OVER () AS eligible,
            row_number() OVER (ORDER BY selection_key) AS position
        FROM (
            SELECT * EXCEPT (eligible, selected, position) FROM scoped
            WHERE (trace_id, trace_ref) IN (
                SELECT JSONExtractString(b.data, 'trace_id'), JSONExtractString(b.data, 'trace_ref')
                FROM lens_state_blobs AS b
                WHERE startsWith(b.key, 'trace-signal/')
                  AND (b.key, b.revision, toString(b.digest)) IN (
                      SELECT key, revision, digest FROM lens_state_heads
                      WHERE startsWith(key, 'trace-signal/')
                  )
                  AND JSONExtractString(b.data, 'config_key') = {signal_config:String}
                  AND JSONExtractString(b.data, 'data', 'status') = 'classified'
                  AND JSONExtractString(b.data, 'data', 'error') = ''
                  AND JSONHas(b.data, 'data', 'scores', {signal_id:String})
                  AND JSONExtractFloat(b.data, 'data', 'scores', {signal_id:String}) BETWEEN {signal_threshold:Float64} AND 1
            )
        )
    )
)
WHERE ({signal_preview:UInt8}=1 OR position <= selected)
  AND selection_key > {signal_after:String}
ORDER BY selection_key LIMIT {signal_limit:UInt32} OFFSET {signal_offset:UInt64}
