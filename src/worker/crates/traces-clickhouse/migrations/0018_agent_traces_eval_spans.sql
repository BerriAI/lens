ALTER TABLE {database}.agent_traces_by_key
    ADD COLUMN IF NOT EXISTS EvalSpanCount SimpleAggregateFunction(sum, UInt64) DEFAULT 0
