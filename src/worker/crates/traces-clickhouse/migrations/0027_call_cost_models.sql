ALTER TABLE {database}.lens_call_costs
    ADD COLUMN IF NOT EXISTS model_id String DEFAULT '',
    ADD COLUMN IF NOT EXISTS model_group String DEFAULT '',
    ADD COLUMN IF NOT EXISTS model String DEFAULT '',
    ADD COLUMN IF NOT EXISTS custom_llm_provider LowCardinality(String) DEFAULT ''
