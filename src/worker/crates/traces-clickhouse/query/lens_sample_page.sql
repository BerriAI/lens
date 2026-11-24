SELECT *, selection_key FROM (
    SELECT *, if({sample_cap:UInt64}=0, ceiling(eligible*{sample_percent:Float64}/100),
        least(toFloat64({sample_cap:UInt64}),ceiling(eligible*{sample_percent:Float64}/100))) AS selected
    FROM (
        SELECT *, count() OVER () AS eligible,
            row_number() OVER (ORDER BY selection_key) AS position
        FROM selected_candidates
    )
)
WHERE ({preview:UInt8}=1 OR position <= selected)
  AND selection_key > {after:String}
ORDER BY selection_key LIMIT {limit:UInt32} OFFSET {offset:UInt64}
