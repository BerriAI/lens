use lens_contract::{feedback::TraceIdentity, investigations::TraceFindingCount};
use lens_investigations::{RepositoryError, TraceFindingsRepository};
use serde::Deserialize;

use super::{Investigations, document, failure};
use crate::state::Head;

const PARENTS: &str = "SELECT s.key AS key, s.revision AS revision, s.digest AS digest, s.version AS version
    FROM (SELECT * FROM lens_schedule FINAL) AS s
    INNER JOIN lens_state_heads AS h USING (key, revision, digest)
    WHERE s.key IN (SELECT parent_key FROM lens_jobs
        WHERE hasAny(trace_ids, JSONExtract({trace_ids:String}, 'Array(String)'))) FORMAT JSONEachRow";

const COUNTS: &str = "WITH
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
            OR (j.key != p.parent_key AND j.archived_version <= p.version
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
GROUP BY trace_id, trace_ref ORDER BY trace_id, trace_ref FORMAT JSONEachRow";

#[derive(Deserialize)]
struct LensHead {
    #[serde(flatten)]
    head: Head,
    version: u64,
}

impl TraceFindingsRepository for Investigations {
    async fn trace_findings(
        &self,
        traces: &[TraceIdentity],
    ) -> Result<Vec<TraceFindingCount>, RepositoryError> {
        let trace_ids = document(
            traces
                .iter()
                .map(|trace| &trace.trace_id)
                .collect::<Vec<_>>(),
        )?
        .to_string();
        let body = self
            .0
            .command(PARENTS, &[("trace_ids", trace_ids.clone())], String::new())
            .await
            .map_err(failure)?;
        let parents: Vec<LensHead> = body
            .lines()
            .map(|line| {
                serde_json::from_str(line)
                    .map_err(|error| RepositoryError::Unavailable(Box::new(error)))
            })
            .collect::<Result<_, _>>()?;
        let references: Vec<_> = parents
            .iter()
            .map(|parent| {
                (
                    &parent.head.key,
                    parent.head.revision,
                    &parent.head.digest,
                    parent.version,
                )
            })
            .collect();
        let targets: Vec<_> = traces
            .iter()
            .map(|trace| (&trace.trace_id, &trace.trace_ref))
            .collect();
        let body = self
            .0
            .command(
                COUNTS,
                &[
                    ("trace_ids", trace_ids),
                    ("traces", document(targets)?.to_string()),
                    ("parents", document(references)?.to_string()),
                ],
                String::new(),
            )
            .await
            .map_err(failure)?;
        body.lines()
            .map(|line| {
                serde_json::from_str(line)
                    .map_err(|error| RepositoryError::Unavailable(Box::new(error)))
            })
            .collect()
    }
}
