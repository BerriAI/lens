ALTER TABLE slack_threads ADD COLUMN sender_id TEXT NOT NULL DEFAULT '';
ALTER TABLE slack_threads ADD COLUMN owner TEXT NOT NULL DEFAULT '';
ALTER TABLE slack_threads ADD COLUMN source TEXT NOT NULL DEFAULT '{}';
CREATE TABLE slack_link_codes (
    digest TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    owner TEXT NOT NULL,
    expires_at INTEGER NOT NULL
);
CREATE TABLE slack_claims (message_id TEXT PRIMARY KEY);
CREATE INDEX slack_history ON slack_messages(conversation_id);
CREATE TABLE slack_deliveries (
    actor_id TEXT NOT NULL REFERENCES actors(id),
    id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    text TEXT NOT NULL,
    receipt TEXT,
    PRIMARY KEY (actor_id,id)
);
