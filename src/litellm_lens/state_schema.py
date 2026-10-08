from typing import Final, LiteralString

INDEX_SCHEMA: Final[tuple[LiteralString, ...]] = (
    """CREATE TABLE IF NOT EXISTS lens_schedule (
        key String, revision UInt64, digest FixedString(64), id String, version UInt64,
        all_teams UInt8, team_id String, api_key_hash String,
        due_at Nullable(DateTime64(6, 'UTC')),
        INDEX due_minmax due_at TYPE minmax GRANULARITY 1
    ) ENGINE=ReplacingMergeTree ORDER BY (team_id, api_key_hash, key, revision, digest)
    SETTINGS fsync_after_insert=1, fsync_part_directory=1""",
    """CREATE MATERIALIZED VIEW IF NOT EXISTS lens_schedule_insert TO lens_schedule AS
        SELECT key, revision, digest, JSONExtractString(data, 'lens', 'id') AS id,
            JSONExtractUInt(data, 'lens', 'version') AS version,
            JSONExtractBool(data, 'lens', 'scope', 'all_teams') AS all_teams,
            JSONExtractString(data, 'lens', 'scope', 'team_id') AS team_id,
            JSONExtractString(data, 'lens', 'scope', 'api_key_hash') AS api_key_hash,
            parseDateTime64BestEffortOrNull(JSONExtractString(data, 'due_at'), 6, 'UTC') AS due_at
        FROM lens_state_blobs WHERE startsWith(key, 'lens/')""",
    """CREATE TABLE IF NOT EXISTS lens_jobs (
        key String, revision UInt64, digest FixedString(64), lens_id String, parent_key String,
        archived_version UInt64, id String, created_at DateTime64(6, 'UTC'), status LowCardinality(String),
        trace_ids Array(String), finding_ids Array(String), job String CODEC(ZSTD(3)),
        INDEX traces_bloom trace_ids TYPE bloom_filter GRANULARITY 1,
        INDEX findings_bloom finding_ids TYPE bloom_filter GRANULARITY 1
    ) ENGINE=ReplacingMergeTree ORDER BY (lens_id, created_at, id, key, revision, digest)
    SETTINGS fsync_after_insert=1, fsync_part_directory=1""",
    """CREATE MATERIALIZED VIEW IF NOT EXISTS lens_jobs_insert TO lens_jobs AS
        SELECT key, revision, digest,
            if(startsWith(key, 'lens/'), JSONExtractString(data, 'lens', 'id'),
                JSONExtractString(data, 'lens_id')) AS lens_id,
            if(startsWith(key, 'lens/'), key, JSONExtractString(data, 'parent_key')) AS parent_key,
            JSONExtractUInt(data, 'archived_version') AS archived_version,
            JSONExtractString(job, 'id') AS id,
            parseDateTime64BestEffort(JSONExtractString(job, 'created_at'), 6, 'UTC') AS created_at,
            JSONExtractString(job, 'status') AS status,
            arrayMap(execution -> JSONExtractString(execution, 'trace_id'),
                arrayFilter(execution -> JSONExtractString(execution, 'source') = 'traces',
                    JSONExtractArrayRaw(job, 'sample', 'executions'))) AS trace_ids,
            arrayMap(finding -> JSONExtractString(finding, 'id'), JSONExtractArrayRaw(job, 'findings')) AS finding_ids,
            job
        FROM lens_state_blobs
        ARRAY JOIN if(startsWith(key, 'lens/'), JSONExtractArrayRaw(data, 'lens', 'jobs'),
            [JSONExtractRaw(data, 'job')]) AS job
        WHERE startsWith(key, 'lens/') OR startsWith(key, 'run/')""",
    """CREATE TABLE IF NOT EXISTS lens_workers (
        key String, revision UInt64, digest FixedString(64), id String,
        all_teams UInt8, team_id String, api_key_hash String, revoked UInt8
    ) ENGINE=ReplacingMergeTree ORDER BY (team_id, api_key_hash, id, key, revision, digest)
    SETTINGS fsync_after_insert=1, fsync_part_directory=1""",
    """CREATE MATERIALIZED VIEW IF NOT EXISTS lens_workers_insert TO lens_workers AS
        SELECT key, revision, digest, JSONExtractString(data, 'id') AS id,
            JSONExtractBool(data, 'scope', 'all_teams') AS all_teams,
            JSONExtractString(data, 'scope', 'team_id') AS team_id,
            JSONExtractString(data, 'scope', 'api_key_hash') AS api_key_hash,
            JSONExtractBool(data, 'revoked') AS revoked
        FROM lens_state_blobs WHERE startsWith(key, 'worker/') AND data != 'null'""",
)
