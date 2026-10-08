from datetime import datetime
from types import MappingProxyType
from typing import Final, LiteralString

from litellm_lens.clickhouse_state import ClickHouseState, Head, json_text
from litellm_lens.models import Job, Lens, Record, Scope, TraceFindingCount, TraceIdentity
from litellm_lens.persistence import select_rows, utc_time


class DueHead(Head):
    due_at: datetime


class LensHead(Head):
    version: int


class FindingRun(Record):
    finding_id: str
    job_id: str


class JobRow(Record):
    job: str


_RUNS: Final[LiteralString] = """WITH jobs AS (
    SELECT j.job AS job, j.id AS id, j.created_at AS created_at
    FROM (SELECT * FROM lens_jobs FINAL WHERE lens_id={lens_id:String}) AS j
    LEFT JOIN lens_state_heads AS h USING (key, revision, digest)
    WHERE (j.key={key:String} AND j.revision={revision:UInt64} AND j.digest={digest:String})
        OR (j.archived_version > 0 AND j.archived_version <= {version:UInt64}
            AND h.key=j.key AND h.revision=j.revision AND h.digest=j.digest)
) """

_TRACE_COUNTS: Final[LiteralString] = """WITH
    targets AS (SELECT arrayJoin(JSONExtract({traces:String}, 'Array(Tuple(String, String))')) AS target),
    parents AS (
        SELECT ref.1 AS parent_key, ref.2 AS revision, ref.3 AS digest, ref.4 AS version
        FROM (SELECT arrayJoin(JSONExtract({parents:String}, 'Array(Tuple(String, UInt64, String, UInt64))')) AS ref)
    ),
    jobs AS (
        SELECT j.job AS job
        FROM (SELECT * FROM lens_jobs FINAL
            WHERE status='completed' AND hasAny(trace_ids, JSONExtract({trace_ids:String}, 'Array(String)'))) AS j
        INNER JOIN parents AS p ON j.parent_key=p.parent_key
        LEFT JOIN lens_state_heads AS h ON j.key=h.key AND j.revision=h.revision AND j.digest=h.digest
        WHERE (j.key=p.parent_key AND j.revision=p.revision AND j.digest=p.digest)
            OR (j.archived_version > 0 AND j.archived_version <= p.version
                AND h.key=j.key AND h.revision=j.revision AND h.digest=j.digest)
    ),
    assessed AS (
        SELECT JSONExtractString(execution, 'trace_id') AS trace_id,
            JSONExtractString(execution, 'trace_ref') AS trace_ref,
            JSONExtractString(execution, 'id') AS execution_id,
            1 AS present,
            arrayMap(finding -> JSONExtractString(finding, 'id'),
                arrayFilter(finding -> has(JSONExtract(finding, 'occurrences', 'Array(String)'), execution_id),
                    JSONExtractArrayRaw(job, 'findings'))) AS finding_ids
        FROM jobs ARRAY JOIN JSONExtractArrayRaw(job, 'sample', 'executions') AS execution
        WHERE JSONExtractString(execution, 'source')='traces'
            AND arrayExists(assessment -> JSONExtractString(assessment, 'execution_id')=execution_id
                AND NOT JSONExtractBool(assessment, 'cannot_assess'), JSONExtractArrayRaw(job, 'assessments'))
    )
SELECT target.1 AS trace_id, target.2 AS trace_ref,
    if(countIf(assessed.present=1)=0, NULL, uniqExactArray(assessed.finding_ids)) AS finding_count
FROM targets LEFT JOIN assessed ON target.1=assessed.trace_id AND target.2=assessed.trace_ref
GROUP BY trace_id, trace_ref ORDER BY trace_id, trace_ref FORMAT JSONEachRow"""


class InvestigationQueries:
    def __init__(self, state: ClickHouseState) -> None:
        self.state: Final = state

    async def due(
        self, scope: Scope, now: datetime, limit: int, after: tuple[datetime, str] | None
    ) -> tuple[DueHead, ...]:
        return await select_rows(
            self.state,
            "SELECT s.key AS key, s.revision AS revision, s.digest AS digest, s.due_at AS due_at "
            "FROM (SELECT * FROM lens_schedule FINAL) AS s "
            "INNER JOIN lens_state_heads AS h USING (key, revision, digest) "
            "WHERE s.due_at IS NOT NULL AND s.due_at <= parseDateTime64BestEffort({now:String}, 6, 'UTC') "
            "AND ({all_teams:Bool}=1 OR (s.all_teams=0 AND s.team_id={team_id:String} "
            "AND ({team_id:String} != '' OR s.api_key_hash={api_key_hash:String}))) "
            "AND ({first:Bool}=1 OR (s.due_at, s.id) > "
            "(parseDateTime64BestEffort({after_time:String}, 6, 'UTC'), {after_id:String})) "
            "ORDER BY s.due_at, s.id LIMIT {limit:UInt32} FORMAT JSONEachRow",
            DueHead,
            MappingProxyType(
                {
                    "now": utc_time(now).isoformat(),
                    "all_teams": str(int(scope.all_teams)),
                    "team_id": scope.team_id,
                    "api_key_hash": scope.api_key_hash,
                    "limit": str(limit),
                    "first": str(int(after is None)),
                    "after_time": utc_time(after[0] if after else now).isoformat(),
                    "after_id": after[1] if after else "",
                }
            ),
        )

    async def jobs(self, head: Head, lens: Lens, offset: int, job_id: str | None = None) -> tuple[Job, ...]:
        rows: Final = await select_rows(
            self.state,
            _RUNS + "SELECT job FROM jobs WHERE {all:Bool}=1 OR id={job_id:String} "
            "ORDER BY created_at DESC, id DESC LIMIT {limit:UInt32} OFFSET {offset:UInt64} FORMAT JSONEachRow",
            JobRow,
            MappingProxyType(
                {
                    "lens_id": lens.id,
                    "key": head.key,
                    "revision": str(head.revision),
                    "digest": head.digest,
                    "version": str(lens.version),
                    "all": str(int(job_id is None)),
                    "job_id": job_id or "",
                    "offset": str(offset),
                    "limit": "50" if job_id is None else "1",
                }
            ),
        )
        return tuple(Job.model_validate_json(row.job) for row in rows)

    async def finding_runs(self, head: Head, lens: Lens, finding_ids: tuple[str, ...]) -> tuple[FindingRun, ...]:
        return await select_rows(
            self.state,
            _RUNS + "SELECT DISTINCT JSONExtractString(finding, 'id') AS finding_id, id AS job_id "
            "FROM jobs ARRAY JOIN JSONExtractArrayRaw(job, 'findings') AS finding "
            "WHERE has(JSONExtract({ids:String}, 'Array(String)'), finding_id) "
            "ORDER BY finding_id, job_id FORMAT JSONEachRow",
            FindingRun,
            MappingProxyType(
                {
                    "lens_id": lens.id,
                    "key": head.key,
                    "revision": str(head.revision),
                    "digest": head.digest,
                    "version": str(lens.version),
                    "ids": json_text(finding_ids),
                }
            ),
        )

    async def trace_findings(self, traces: tuple[TraceIdentity, ...]) -> tuple[TraceFindingCount, ...]:
        trace_ids: Final = json_text(tuple(trace.trace_id for trace in traces))
        parents: Final = await select_rows(
            self.state,
            "SELECT s.key AS key, s.revision AS revision, s.digest AS digest, s.version AS version "
            "FROM (SELECT * FROM lens_schedule FINAL) AS s "
            "INNER JOIN lens_state_heads AS h USING (key, revision, digest) "
            "WHERE s.key IN (SELECT parent_key FROM lens_jobs "
            "WHERE hasAny(trace_ids, JSONExtract({trace_ids:String}, 'Array(String)'))) FORMAT JSONEachRow",
            LensHead,
            MappingProxyType({"trace_ids": trace_ids}),
        )
        return await select_rows(
            self.state,
            _TRACE_COUNTS,
            TraceFindingCount,
            MappingProxyType(
                {
                    "trace_ids": trace_ids,
                    "traces": json_text(tuple((trace.trace_id, trace.trace_ref) for trace in traces)),
                    "parents": json_text(tuple((p.key, p.revision, p.digest, p.version) for p in parents)),
                }
            ),
        )
