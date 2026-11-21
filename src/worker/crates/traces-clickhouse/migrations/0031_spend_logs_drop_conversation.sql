ALTER TABLE {database}.spend_logs
    DROP COLUMN IF EXISTS messages,
    DROP COLUMN IF EXISTS response
