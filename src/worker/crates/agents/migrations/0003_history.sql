CREATE TABLE native_history (
    conversation_id TEXT PRIMARY KEY REFERENCES conversations(id),
    messages TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
