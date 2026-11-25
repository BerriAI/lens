CREATE TABLE artifacts (
    workspace TEXT NOT NULL,
    id TEXT NOT NULL,
    producer_id TEXT NOT NULL,
    filename TEXT NOT NULL,
    media_type TEXT NOT NULL,
    content BLOB NOT NULL,
    digest TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY(workspace,id)
);
CREATE TABLE report_artifacts (
    workspace TEXT NOT NULL,
    report_id TEXT NOT NULL,
    artifact_id TEXT NOT NULL,
    position INTEGER NOT NULL,
    PRIMARY KEY(workspace,report_id,artifact_id),
    FOREIGN KEY(workspace,report_id) REFERENCES reports(workspace,id) ON DELETE CASCADE,
    FOREIGN KEY(workspace,artifact_id) REFERENCES artifacts(workspace,id)
);
