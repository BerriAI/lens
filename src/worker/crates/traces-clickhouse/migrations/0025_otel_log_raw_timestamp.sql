ALTER TABLE {database}.otel_logs
ADD COLUMN IF NOT EXISTS RawTimestamp DateTime64(9) DEFAULT Timestamp
AFTER Timestamp
