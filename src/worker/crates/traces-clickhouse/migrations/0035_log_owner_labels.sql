ALTER TABLE {database}.otel_logs
    ADD COLUMN IF NOT EXISTS KeyAlias String DEFAULT '',
    ADD COLUMN IF NOT EXISTS UserEmail String DEFAULT ''
