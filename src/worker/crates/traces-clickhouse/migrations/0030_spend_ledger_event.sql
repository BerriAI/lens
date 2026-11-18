ALTER TABLE {database}.spend_logs
    ADD COLUMN IF NOT EXISTS ledger_event_id String DEFAULT '' AFTER request_id
