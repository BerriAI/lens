CREATE TABLE IF NOT EXISTS "LensSession" (
    id TEXT PRIMARY KEY,
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS "LensSession_expires_at_idx" ON "LensSession" (expires_at);

CREATE TABLE IF NOT EXISTS "LensAnalysisConnection" (
    id TEXT PRIMARY KEY,
    data JSONB NOT NULL
);
