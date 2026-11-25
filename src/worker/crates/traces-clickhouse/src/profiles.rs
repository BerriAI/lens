use litellm_http::Client;
use litellm_storage_clickhouse::{Query, ReadLimits, fetch, insert_encoded_rows};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{Config, Error};

const LIMITS: ReadLimits = ReadLimits {
    result_rows: 65,
    response_bytes: 16 * 1024,
    execution_seconds: 3,
};

const MAX_ROW_BYTES: usize = 16 * 1024;

fn encoded(row: &impl Serialize) -> Result<String, Error> {
    let body = serde_json::to_string(row).map_err(|_| Error::InvalidRow)?;
    if body.len() > MAX_ROW_BYTES {
        return Err(Error::InsertTooLarge);
    }
    Ok(body)
}

async fn insert(
    client: &Client,
    config: &Config,
    table: &str,
    row: &impl Serialize,
) -> Result<(), Error> {
    let body = encoded(row)?;
    let token = format!("{:x}", Sha256::digest(body.as_bytes()));
    insert_encoded_rows(
        client,
        config.storage().writer(),
        config.storage().database(),
        table,
        &token,
        &body,
    )
    .await?;
    Ok(())
}

#[derive(Clone, Serialize)]
pub struct BucketingParams {
    pub agent: String,
    pub taxonomy_id: String,
    pub taxonomy_version: u32,
    pub start_ms: i64,
    pub end_ms: i64,
    pub window_starts: Vec<i64>,
    pub window_ends: Vec<i64>,
    pub require_input: u8,
    pub max_attempts: u8,
    pub limit: u32,
}

impl BucketingParams {
    /// Unrecorded runs of `agent` with an input, starting in any `[start, end)` window, newest
    /// first. The discovery preview count and the discovery sample both select runs this way.
    pub fn windows(agent: &str, windows: &[(i64, i64)], limit: u32) -> Self {
        Self {
            agent: agent.to_owned(),
            taxonomy_id: String::new(),
            taxonomy_version: 0,
            start_ms: windows.iter().map(|window| window.0).min().unwrap_or(0),
            end_ms: windows.iter().map(|window| window.1).max().unwrap_or(0),
            window_starts: windows.iter().map(|window| window.0).collect(),
            window_ends: windows.iter().map(|window| window.1).collect(),
            require_input: 1,
            max_attempts: 0,
            limit,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct BucketingCandidate {
    pub team_id: String,
    pub api_key_hash: String,
    pub trace_id: String,
    pub user_ids: Vec<String>,
    #[serde(deserialize_with = "crate::query::number::deserialize")]
    pub start_ms: i64,
    pub input_preview: String,
    #[serde(default, deserialize_with = "crate::query::number::deserialize")]
    pub attempts: u8,
}

/// Assignments expire with their run's start like every trace table, so runs past retention are
/// never sampled or classified; an assignment for one would vanish and be paid for again.
#[derive(Serialize)]
struct Retained {
    #[serde(flatten)]
    params: BucketingParams,
    retention_days: u32,
}

fn retained(config: &Config, params: &BucketingParams) -> Retained {
    Retained {
        params: params.clone(),
        retention_days: config.retention_days(),
    }
}

struct BucketingCandidates;

impl Query for BucketingCandidates {
    type Params = Retained;
    type Row = BucketingCandidate;
    const READ_LIMITS: ReadLimits = ReadLimits {
        result_rows: 101,
        response_bytes: 512 * 1024,
        execution_seconds: 10,
    };
    const SQL: &'static str = concat!(
        include_str!("../query/profile_selection.sql"),
        include_str!("../query/profile_bucketing_candidates.sql")
    );
}

struct DiscoverySamples;

impl Query for DiscoverySamples {
    type Params = Retained;
    type Row = BucketingCandidate;
    const READ_LIMITS: ReadLimits = ReadLimits {
        result_rows: 501,
        response_bytes: 2 * 1024 * 1024,
        execution_seconds: 20,
    };
    const SQL: &'static str = concat!(
        include_str!("../query/profile_selection.sql"),
        include_str!("../query/profile_bucketing_candidates.sql")
    );
}

#[derive(Deserialize)]
struct EligibleRuns {
    #[serde(deserialize_with = "crate::query::number::deserialize")]
    eligible_runs: u64,
}

struct DiscoveryEligible;

impl Query for DiscoveryEligible {
    type Params = Retained;
    type Row = EligibleRuns;
    const READ_LIMITS: ReadLimits = ReadLimits {
        result_rows: 1,
        response_bytes: 4 * 1024,
        execution_seconds: 20,
    };
    const SQL: &'static str = concat!(
        include_str!("../query/profile_selection.sql"),
        include_str!("../query/profile_selection_count.sql")
    );
}

/// How many runs discovery could sample with these parameters, ignoring the limit.
pub async fn discovery_eligible(
    client: &Client,
    config: &Config,
    params: &BucketingParams,
) -> Result<u64, Error> {
    Ok(
        fetch::<DiscoveryEligible>(client, config.storage().reader(), &retained(config, params))
            .await?
            .first()
            .map_or(0, |row| row.eligible_runs),
    )
}

/// Unrecorded runs of the agent in the discovery window, newest first.
pub async fn discovery_samples(
    client: &Client,
    config: &Config,
    params: &BucketingParams,
) -> Result<Vec<BucketingCandidate>, Error> {
    Ok(
        fetch::<DiscoverySamples>(client, config.storage().reader(), &retained(config, params))
            .await?,
    )
}

/// Unrecorded runs of the agent that this taxonomy version has not settled yet, newest first.
pub async fn bucketing_candidates(
    client: &Client,
    config: &Config,
    params: &BucketingParams,
) -> Result<Vec<BucketingCandidate>, Error> {
    Ok(
        fetch::<BucketingCandidates>(client, config.storage().reader(), &retained(config, params))
            .await?,
    )
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentOutcome {
    Assigned,
    Unknown,
    Failed,
}

/// The exact row an assignment is stored as. Replaying it is idempotent: the insert token is
/// its hash, and every version of it shares a sorting key, `StartTs` month and `ComputedAt`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AssignmentRecord {
    pub team_id: String,
    pub api_key_hash: String,
    pub trace_id: String,
    pub user_ids: Vec<String>,
    pub agent_name: String,
    pub taxonomy_id: String,
    pub taxonomy_version: u32,
    pub task_id: String,
    pub outcome: AssignmentOutcome,
    pub attempts: u8,
    pub start_ts: String,
    pub computed_at: u64,
}

pub struct TaskAssignment<'a> {
    pub candidate: &'a BucketingCandidate,
    pub agent: &'a str,
    pub taxonomy_id: &'a str,
    pub taxonomy_version: u32,
    pub task_id: &'a str,
    pub outcome: AssignmentOutcome,
}

impl AssignmentRecord {
    pub fn insertable(&self) -> bool {
        encoded(self).is_ok()
    }
}

impl TaskAssignment<'_> {
    pub fn record(&self, start_ms: i64, computed_at: u64) -> AssignmentRecord {
        AssignmentRecord {
            team_id: self.candidate.team_id.clone(),
            api_key_hash: self.candidate.api_key_hash.clone(),
            trace_id: self.candidate.trace_id.clone(),
            user_ids: self.candidate.user_ids.clone(),
            agent_name: self.agent.to_owned(),
            taxonomy_id: self.taxonomy_id.to_owned(),
            taxonomy_version: self.taxonomy_version,
            task_id: self.task_id.to_owned(),
            outcome: self.outcome,
            attempts: self.candidate.attempts.saturating_add(1),
            start_ts: litellm_traces::iso_time(start_ms),
            computed_at,
        }
    }
}

pub async fn record_assignment(
    client: &Client,
    config: &Config,
    record: &AssignmentRecord,
) -> Result<(), Error> {
    insert(
        client,
        config,
        "lens_task_profile_assignments_monthly",
        record,
    )
    .await
}

#[derive(Serialize)]
struct RunKey {
    team_id: String,
    api_key_hash: String,
    trace_id: String,
}

#[derive(Deserialize)]
struct StoredStart {
    #[serde(deserialize_with = "crate::query::number::deserialize")]
    start_ms: i64,
}

struct AssignmentStart;

impl Query for AssignmentStart {
    type Params = RunKey;
    type Row = StoredStart;
    const READ_LIMITS: ReadLimits = LIMITS;
    const SQL: &'static str = "SELECT toUnixTimestamp64Milli(min(StartTs)) AS start_ms \
        FROM lens_task_profile_assignment_rows \
        WHERE TeamId={team_id:String} AND ApiKeyHash={api_key_hash:String} \
          AND TraceId={trace_id:String} \
        HAVING count() > 0";
}

pub async fn assignment_start(
    client: &Client,
    config: &Config,
    candidate: &BucketingCandidate,
) -> Result<Option<i64>, Error> {
    Ok(fetch::<AssignmentStart>(
        client,
        config.storage().reader(),
        &RunKey {
            team_id: candidate.team_id.clone(),
            api_key_hash: candidate.api_key_hash.clone(),
            trace_id: candidate.trace_id.clone(),
        },
    )
    .await?
    .first()
    .map(|row| row.start_ms))
}

#[cfg(test)]
mod tests {
    use super::{
        AssignmentOutcome, AssignmentRecord, BucketingCandidate, MAX_ROW_BYTES, TaskAssignment,
        encoded,
    };
    use rstest::rstest;

    fn with_user(user: String) -> AssignmentRecord {
        TaskAssignment {
            candidate: &BucketingCandidate {
                team_id: "team".into(),
                api_key_hash: "key".into(),
                trace_id: "trace".into(),
                user_ids: vec![user],
                start_ms: 1,
                input_preview: String::new(),
                attempts: 0,
            },
            agent: "agent",
            taxonomy_id: "tax",
            taxonomy_version: 1,
            task_id: "task",
            outcome: AssignmentOutcome::Assigned,
        }
        .record(1, 1)
    }

    #[rstest]
    #[case::at_the_limit(0, true)]
    #[case::one_byte_over(1, false)]
    fn an_assignment_is_insertable_only_within_the_row_limit(
        #[case] over: usize,
        #[case] expected: bool,
    ) {
        let small = encoded(&with_user(String::new())).expect("small row");
        let record = with_user("u".repeat(MAX_ROW_BYTES - small.len() + over));

        assert_eq!(record.insertable(), expected);
    }
}
