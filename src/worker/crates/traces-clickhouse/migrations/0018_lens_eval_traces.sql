CREATE TABLE IF NOT EXISTS {database}.lens_eval_traces
(
    TeamId     LowCardinality(String),
    ApiKeyHash String,
    TraceId    String,
    StartTs    SimpleAggregateFunction(min, DateTime64(9)),
    UserIds    SimpleAggregateFunction(groupUniqArrayArray, Array(String)),
    RequestIds SimpleAggregateFunction(groupUniqArrayArray, Array(String))
)
ENGINE = AggregatingMergeTree
ORDER BY (TeamId, ApiKeyHash, TraceId)
SETTINGS materialize_ttl_recalculate_only = 1, non_replicated_deduplication_window = 1000
