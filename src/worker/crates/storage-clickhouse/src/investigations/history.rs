use lens_contract::{investigations::FindingRun, worker::Job};
use lens_investigations::RepositoryError;
use serde::Deserialize;

use super::{Investigations, StoredLens, decode, failure};

const HISTORY: &str = "WITH jobs AS (
    SELECT j.job AS job, j.id AS id, j.created_at AS created_at
    FROM (SELECT * FROM lens_jobs FINAL WHERE lens_id={lens_id:String}) AS j
    LEFT JOIN lens_state_heads AS h USING (key, revision, digest)
    WHERE (j.key={key:String} AND j.revision={revision:UInt64} AND j.digest={digest:String})
        OR (j.key != {key:String} AND j.archived_version <= {version:UInt64}
            AND h.key=j.key AND h.revision=j.revision AND h.digest=j.digest)
) SELECT job FROM jobs WHERE {all:Bool}=1 OR id={job_id:String}
ORDER BY created_at DESC, id DESC LIMIT {limit:UInt32} OFFSET {offset:UInt64} FORMAT JSONEachRow";

const FINDING_RUNS: &str = "WITH jobs AS (
    SELECT j.job AS job, j.id AS id
    FROM (SELECT * FROM lens_jobs FINAL WHERE lens_id={lens_id:String}) AS j
    LEFT JOIN lens_state_heads AS h USING (key, revision, digest)
    WHERE (j.key={key:String} AND j.revision={revision:UInt64} AND j.digest={digest:String})
        OR (j.key != {key:String} AND j.archived_version <= {version:UInt64}
            AND h.key=j.key AND h.revision=j.revision AND h.digest=j.digest)
) SELECT DISTINCT JSONExtractString(finding, 'id') AS finding_id, id AS job_id
FROM jobs ARRAY JOIN JSONExtractArrayRaw(job, 'findings') AS finding
WHERE has(JSONExtract({ids:String}, 'Array(String)'), finding_id)
ORDER BY finding_id, job_id FORMAT JSONEachRow";

#[derive(Deserialize)]
struct JobRow {
    job: String,
}

impl Investigations {
    pub async fn finding_runs(
        &self,
        lens_id: &str,
        finding_ids: &[String],
    ) -> Result<Vec<FindingRun>, RepositoryError> {
        if finding_ids.is_empty() {
            return Ok(Vec::new());
        }
        let snapshot = self.snapshot(lens_id).await?;
        if snapshot.value.is_null() {
            return Ok(Vec::new());
        }
        let stored: StoredLens = decode(snapshot.value)?;
        let ids = serde_json::to_string(finding_ids)
            .map_err(|error| RepositoryError::Unavailable(Box::new(error)))?;
        let body = self
            .0
            .command(
                FINDING_RUNS,
                &[
                    ("lens_id", stored.lens.id),
                    ("key", snapshot.head.key),
                    ("revision", snapshot.head.revision.to_string()),
                    ("digest", snapshot.head.digest),
                    ("version", stored.lens.version.to_string()),
                    ("ids", ids),
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

    pub(super) async fn history(
        &self,
        lens_id: &str,
        offset: u64,
        job_id: Option<&str>,
    ) -> Result<Vec<Job>, RepositoryError> {
        let snapshot = self.snapshot(lens_id).await?;
        if snapshot.value.is_null() {
            return Ok(Vec::new());
        }
        let stored: StoredLens = decode(snapshot.value)?;
        let body = self
            .0
            .command(
                HISTORY,
                &[
                    ("lens_id", stored.lens.id),
                    ("key", snapshot.head.key),
                    ("revision", snapshot.head.revision.to_string()),
                    ("digest", snapshot.head.digest),
                    ("version", stored.lens.version.to_string()),
                    ("all", u8::from(job_id.is_none()).to_string()),
                    ("job_id", job_id.unwrap_or_default().to_owned()),
                    ("offset", offset.to_string()),
                    ("limit", if job_id.is_none() { "50" } else { "1" }.into()),
                ],
                String::new(),
            )
            .await
            .map_err(failure)?;
        body.lines()
            .map(|line| {
                let row: JobRow = serde_json::from_str(line)
                    .map_err(|error| RepositoryError::Unavailable(Box::new(error)))?;
                serde_json::from_str(&row.job)
                    .map_err(|error| RepositoryError::Unavailable(Box::new(error)))
            })
            .collect()
    }
}
