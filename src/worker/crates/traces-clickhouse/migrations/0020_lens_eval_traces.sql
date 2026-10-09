CREATE VIEW IF NOT EXISTS {database}.lens_eval_traces AS
SELECT TeamId, ApiKeyHash, TraceId FROM {database}.agent_traces_by_key WHERE EvalSpanCount > 0
