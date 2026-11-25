CREATE TABLE actors (
    uid INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace TEXT NOT NULL,
    id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    instructions TEXT NOT NULL,
    runtime TEXT NOT NULL,
    owner TEXT NOT NULL,
    scope TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL
);
CREATE INDEX actors_workspace ON actors(workspace, runtime, created_at);
CREATE UNIQUE INDEX main_agent_owner ON actors(workspace, owner) WHERE runtime='deepagents';
CREATE TABLE conversations (
    id TEXT PRIMARY KEY,
    actor_id TEXT NOT NULL REFERENCES actors(id),
    title TEXT NOT NULL,
    native_id TEXT,
    permanent INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX permanent_bot_chat ON conversations(actor_id) WHERE permanent=1;
CREATE TABLE turns (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id),
    input TEXT NOT NULL,
    status TEXT NOT NULL,
    error TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    turn_id TEXT NOT NULL REFERENCES turns(id),
    kind TEXT NOT NULL,
    data TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE INDEX events_turn ON events(turn_id, sequence);
CREATE TABLE slack_threads (
    conversation_id TEXT PRIMARY KEY REFERENCES conversations(id),
    slack_conversation_id TEXT NOT NULL UNIQUE
);
CREATE TABLE slack_messages (
    message_id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    data TEXT NOT NULL
);
