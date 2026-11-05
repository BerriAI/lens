ALTER TABLE {database}.otel_logs
ADD INDEX IF NOT EXISTS idx_severity SeverityText TYPE set(256) GRANULARITY 1
