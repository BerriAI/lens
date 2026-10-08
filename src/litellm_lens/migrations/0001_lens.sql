CREATE TABLE IF NOT EXISTS "LiteLLM_Lens" (
    id TEXT PRIMARY KEY,
    version INTEGER NOT NULL DEFAULT 0,
    data JSONB NOT NULL,
    due_at TIMESTAMP(3) DEFAULT '1970-01-01 00:00:00'
);

CREATE TABLE IF NOT EXISTS "LiteLLM_LensRun" (
    id TEXT PRIMARY KEY,
    lens_id TEXT NOT NULL,
    created_at TIMESTAMP(3) NOT NULL,
    data JSONB NOT NULL
);

CREATE TABLE IF NOT EXISTS "LiteLLM_LensReview" (
    lens_id TEXT NOT NULL,
    criteria_key TEXT NOT NULL,
    execution_id TEXT NOT NULL,
    data JSONB NOT NULL,
    PRIMARY KEY (lens_id, criteria_key, execution_id),
    CONSTRAINT "LiteLLM_LensReview_lens_id_fkey" FOREIGN KEY (lens_id)
        REFERENCES "LiteLLM_Lens"(id) ON DELETE CASCADE ON UPDATE CASCADE
);

CREATE TABLE IF NOT EXISTS "LiteLLM_LensWorker" (
    id TEXT PRIMARY KEY,
    token_hash TEXT NOT NULL UNIQUE,
    data JSONB NOT NULL
);

CREATE TABLE IF NOT EXISTS "LiteLLM_LensIngestionKey" (
    id TEXT PRIMARY KEY,
    data JSONB NOT NULL
);

CREATE TABLE IF NOT EXISTS "LiteLLM_LensDataset" (
    id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    created_at TIMESTAMP(3) NOT NULL,
    data JSONB NOT NULL,
    PRIMARY KEY (id, revision)
);

CREATE TABLE IF NOT EXISTS "LiteLLM_LensSignalConfig" (
    id TEXT PRIMARY KEY,
    data JSONB NOT NULL
);

CREATE TABLE IF NOT EXISTS "LiteLLM_LensTraceSignal" (
    trace_id TEXT NOT NULL,
    trace_ref TEXT NOT NULL DEFAULT '',
    config_key TEXT NOT NULL,
    span_count INTEGER NOT NULL,
    claimed_until TIMESTAMP(3),
    classified_at TIMESTAMP(3),
    data JSONB NOT NULL,
    PRIMARY KEY (trace_id, trace_ref)
);

CREATE INDEX IF NOT EXISTS "LiteLLM_Lens_due_at_idx" ON "LiteLLM_Lens" (due_at);
CREATE INDEX IF NOT EXISTS "LiteLLM_LensRun_lens_id_created_at_idx" ON "LiteLLM_LensRun" (lens_id, created_at);
CREATE INDEX IF NOT EXISTS "LiteLLM_LensWorker_active_scope_idx"
    ON "LiteLLM_LensWorker" USING GIN ((data->'scope') jsonb_path_ops)
    WHERE data @> '{"revoked": false}'::jsonb;
CREATE INDEX IF NOT EXISTS "LiteLLM_LensRun_completed_executions_idx"
    ON "LiteLLM_LensRun" USING GIN ((data->'sample'->'executions') jsonb_path_ops)
    WHERE data->>'status'='completed';
CREATE INDEX IF NOT EXISTS "LiteLLM_Lens_jobs_idx"
    ON "LiteLLM_Lens" USING GIN ((data->'jobs') jsonb_path_ops);
