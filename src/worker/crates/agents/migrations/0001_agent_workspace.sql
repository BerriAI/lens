CREATE TABLE definitions (
    workspace TEXT NOT NULL,
    id TEXT NOT NULL,
    definition TEXT NOT NULL,
    created_by TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (workspace, id)
);
CREATE TABLE definition_names (
    workspace TEXT NOT NULL,
    name TEXT NOT NULL,
    definition_id TEXT NOT NULL,
    PRIMARY KEY (workspace, name),
    FOREIGN KEY (workspace, definition_id) REFERENCES definitions(workspace, id)
);
CREATE INDEX definition_aliases ON definition_names(workspace, definition_id);
CREATE TABLE annotations (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    trace_ref TEXT NOT NULL,
    producer_id TEXT NOT NULL,
    definition_id TEXT NOT NULL,
    value TEXT NOT NULL,
    evidence TEXT NOT NULL,
    execution_id TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (workspace, trace_id, trace_ref, producer_id, definition_id),
    FOREIGN KEY (workspace, definition_id) REFERENCES definitions(workspace, id)
);
CREATE INDEX annotations_filter ON annotations(workspace, definition_id, producer_id, value, sequence);
CREATE INDEX annotations_trace ON annotations(workspace, trace_id, trace_ref, sequence);
CREATE TABLE reports (
    workspace TEXT NOT NULL,
    id TEXT NOT NULL,
    producer_id TEXT NOT NULL,
    execution_id TEXT NOT NULL,
    title TEXT NOT NULL,
    markdown TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (workspace, id)
);
CREATE TABLE report_traces (
    workspace TEXT NOT NULL,
    report_id TEXT NOT NULL,
    trace_id TEXT NOT NULL,
    trace_ref TEXT NOT NULL,
    PRIMARY KEY (workspace, report_id, trace_id, trace_ref),
    FOREIGN KEY (workspace, report_id) REFERENCES reports(workspace, id) ON DELETE CASCADE
);
CREATE INDEX reports_trace ON report_traces(workspace, trace_id, trace_ref);
