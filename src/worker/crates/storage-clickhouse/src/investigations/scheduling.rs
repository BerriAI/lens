use chrono::{DateTime, Utc};
use lens_contract::investigations::{Lens, Scope};
use lens_investigations::{DueLens, RepositoryError, ScheduleRepository, due_at};
use serde::Deserialize;

use super::{Investigations, StoredLens, decode, failure, stored};
use crate::state::{Change, Head};

const DUE: &str = "SELECT s.key AS key, s.revision AS revision, s.digest AS digest,
    formatDateTime(s.due_at, '%Y-%m-%dT%H:%i:%S.%fZ', 'UTC') AS due_at
    FROM (SELECT * FROM lens_schedule FINAL) AS s
    INNER JOIN lens_state_heads AS h USING (key, revision, digest)
    WHERE s.due_at IS NOT NULL AND s.due_at <= parseDateTime64BestEffort({now:String}, 6, 'UTC')
    AND ({all_teams:Bool}=1 OR (s.all_teams=0 AND s.team_id={team_id:String}
        AND ({team_id:String} != '' OR s.api_key_hash={api_key_hash:String})))
    AND ({first:Bool}=1 OR (s.due_at, s.id) >
        (parseDateTime64BestEffort({after_time:String}, 6, 'UTC'), {after_id:String}))
    ORDER BY s.due_at, s.id LIMIT {limit:UInt32} FORMAT JSONEachRow";

#[derive(Deserialize)]
struct DueHead {
    #[serde(flatten)]
    head: Head,
    due_at: DateTime<Utc>,
}

impl ScheduleRepository for Investigations {
    async fn due(
        &self,
        scope: &Scope,
        now: &DateTime<Utc>,
        limit: u32,
        after: Option<&DueLens>,
    ) -> Result<Vec<DueLens>, RepositoryError> {
        let body = self
            .0
            .command(
                DUE,
                &[
                    ("now", now.to_rfc3339()),
                    ("all_teams", u8::from(scope.all_teams).to_string()),
                    ("team_id", scope.team_id.clone()),
                    ("api_key_hash", scope.api_key_hash.clone()),
                    ("limit", limit.to_string()),
                    ("first", u8::from(after.is_none()).to_string()),
                    (
                        "after_time",
                        after.map_or(*now, |entry| entry.due_at).to_rfc3339(),
                    ),
                    (
                        "after_id",
                        after.map_or_else(String::new, |entry| entry.lens.id.clone()),
                    ),
                ],
                String::new(),
            )
            .await
            .map_err(failure)?;
        let rows: Vec<DueHead> = body
            .lines()
            .map(|line| {
                serde_json::from_str(line)
                    .map_err(|error| RepositoryError::Unavailable(Box::new(error)))
            })
            .collect::<Result<_, _>>()?;
        let heads: Vec<_> = rows.iter().map(|row| row.head.clone()).collect();
        let records = self.0.resolve(&heads).await.map_err(failure)?;
        rows.into_iter()
            .zip(records)
            .map(|(row, record)| {
                Ok(DueLens {
                    lens: decode::<StoredLens>(record.value)?.lens,
                    due_at: row.due_at,
                })
            })
            .collect()
    }

    async fn sync_due(&self, lens: &Lens) -> Result<(), RepositoryError> {
        let writer = self.writer(&lens.id).await?;
        let previous = self.snapshot(&lens.id).await?;
        if previous.value.is_null() {
            return Ok(());
        }
        let current: StoredLens = decode(previous.value.clone())?;
        if current.lens.version != lens.version || current.due_at == due_at(&current.lens) {
            return Ok(());
        }
        let corrected = stored(&current.lens)?;
        match writer
            .commit(vec![Change {
                previous,
                value: corrected,
            }])
            .await
        {
            Ok(()) | Err(RepositoryError::Conflict) => Ok(()),
            Err(error) => Err(error),
        }
    }
}
