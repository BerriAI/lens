use lens_profiles::{Assignment, Classifier, TaskCluster};
use litellm_http::Client;
use litellm_storage_clickhouse::{Query, ReadLimits, fetch, insert_encoded_rows};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use crate::{Config, Error};

pub const RESOLUTION_VERSION: u32 = 1;
const LIMITS: ReadLimits = ReadLimits {
    result_rows: 65,
    response_bytes: 16 * 1024,
    execution_seconds: 3,
};

#[derive(Serialize)]
struct CandidateParams {
    version: u32,
}

#[derive(Deserialize)]
struct Candidate {
    team_id: String,
    api_key_hash: String,
    trace_id: String,
    agent_name: String,
    service_name: String,
    user_ids: Vec<String>,
    #[serde(deserialize_with = "crate::query::number::deserialize")]
    start_ms: i64,
}

struct Candidates;

impl Query for Candidates {
    type Params = CandidateParams;
    type Row = Candidate;
    const READ_LIMITS: ReadLimits = LIMITS;
    const SQL: &'static str = include_str!("../query/task_profile_candidates.sql");
}

#[derive(Serialize)]
struct Scope {
    team_id: String,
    api_key_hash: String,
    trace_id: String,
    agent_name: String,
    service_name: String,
    version: u32,
    ownership_hash: String,
}

impl From<&Candidate> for Scope {
    fn from(candidate: &Candidate) -> Self {
        Self {
            team_id: candidate.team_id.clone(),
            api_key_hash: candidate.api_key_hash.clone(),
            trace_id: candidate.trace_id.clone(),
            agent_name: candidate.agent_name.clone(),
            service_name: candidate.service_name.clone(),
            version: RESOLUTION_VERSION,
            ownership_hash: ownership_hash(&candidate.user_ids),
        }
    }
}

#[derive(Deserialize)]
struct TaskInput {
    input: String,
    tool: String,
    recorded: String,
}

struct Inputs;

impl Query for Inputs {
    type Params = Scope;
    type Row = TaskInput;
    const READ_LIMITS: ReadLimits = LIMITS;
    const SQL: &'static str = include_str!("../query/task_profile_input.sql");
}

struct Clusters;

impl Query for Clusters {
    type Params = Scope;
    type Row = TaskCluster;
    const READ_LIMITS: ReadLimits = ReadLimits {
        response_bytes: 512 * 1024,
        ..LIMITS
    };
    const SQL: &'static str = "SELECT ClusterId AS id, Category AS label, Embedding AS embedding \
        FROM lens_task_clusters FINAL \
        WHERE TeamId={team_id:String} AND ApiKeyHash={api_key_hash:String} \
          AND OwnershipHash={ownership_hash:String} AND AgentName={agent_name:String} AND ServiceName={service_name:String} \
          AND ResolutionVersion={version:UInt32} \
        ORDER BY ClusterId LIMIT 65";
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct ProfileRow<'a> {
    team_id: &'a str,
    api_key_hash: &'a str,
    trace_id: &'a str,
    user_ids: &'a [String],
    agent_name: &'a str,
    service_name: &'a str,
    cluster_id: &'a str,
    category: &'a str,
    start_ts: String,
    computed_at: u64,
    resolution_version: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct StoredCluster<'a> {
    ownership_hash: &'a str,
    team_id: &'a str,
    api_key_hash: &'a str,
    agent_name: &'a str,
    service_name: &'a str,
    cluster_id: &'a str,
    category: &'a str,
    embedding: &'a [f32],
    updated_at: String,
    computed_at: u64,
    resolution_version: u32,
}

fn ownership_hash(users: &[String]) -> String {
    let mut users: Vec<_> = users.iter().map(String::as_str).collect();
    users.sort_unstable();
    users.dedup();
    let mut hash = Sha256::new();
    for user in users {
        hash.update((user.len() as u64).to_be_bytes());
        hash.update(user.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

fn descriptor(inputs: &[TaskInput]) -> String {
    let mut text = String::new();
    for input in inputs.iter().take(8) {
        for value in [&input.input, &input.tool] {
            for character in value.chars() {
                if text.len() + character.len_utf8() >= lens_profiles::MAX_TASK_BYTES {
                    return text;
                }
                text.push(character);
            }
            if text.len() >= lens_profiles::MAX_TASK_BYTES {
                return text;
            }
            text.push(' ');
        }
    }
    text
}

async fn insert(
    client: &Client,
    config: &Config,
    table: &str,
    row: &impl Serialize,
) -> Result<(), Error> {
    let body = serde_json::to_string(row).map_err(|_| Error::InvalidRow)?;
    if body.len() > 16 * 1024 {
        return Err(Error::InsertTooLarge);
    }
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

pub async fn project(
    client: &Client,
    config: &Config,
    classifier: &mut Classifier,
) -> Result<bool, Error> {
    let mut url = config.storage().reader().url().clone();
    let pairs: Vec<_> = url
        .query_pairs()
        .filter(|(name, _)| !matches!(name.as_ref(), "max_threads" | "max_memory_usage"))
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    url.query_pairs_mut()
        .clear()
        .extend_pairs(pairs)
        .append_pair("max_threads", "1")
        .append_pair("max_memory_usage", "67108864");
    let reader = crate::Connection::parse(url.as_str())?;
    let candidates = fetch::<Candidates>(
        client,
        &reader,
        &CandidateParams {
            version: RESOLUTION_VERSION,
        },
    )
    .await?;
    let Some(candidate) = candidates.first() else {
        return Ok(false);
    };
    let scope = Scope::from(candidate);
    let inputs = fetch::<Inputs>(client, &reader, &scope).await?;
    let recorded = inputs.iter().find(|row| !row.recorded.is_empty());
    let mut clusters = Vec::new();
    let assignment = if let Some(recorded) = recorded {
        Assignment {
            cluster_id: String::new(),
            label: recorded.recorded.chars().take(256).collect(),
            new_cluster: None,
        }
    } else {
        let rows = fetch::<Clusters>(client, &reader, &scope).await?;
        clusters.extend(rows);
        classifier.classify(&descriptor(&inputs), &clusters)?
    };
    let now = (OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as u64;
    let cluster = assignment.new_cluster.as_ref().or_else(|| {
        clusters
            .iter()
            .find(|cluster| cluster.id == assignment.cluster_id)
    });
    if let Some(cluster) = cluster {
        insert(
            client,
            config,
            "lens_task_clusters",
            &StoredCluster {
                ownership_hash: &scope.ownership_hash,
                team_id: &candidate.team_id,
                api_key_hash: &candidate.api_key_hash,
                agent_name: &candidate.agent_name,
                service_name: &candidate.service_name,
                cluster_id: &cluster.id,
                category: &cluster.label,
                embedding: &cluster.embedding,
                updated_at: litellm_traces::iso_time(now as i64),
                computed_at: now,
                resolution_version: RESOLUTION_VERSION,
            },
        )
        .await?;
    }
    insert(
        client,
        config,
        "lens_task_profiles",
        &ProfileRow {
            team_id: &candidate.team_id,
            api_key_hash: &candidate.api_key_hash,
            trace_id: &candidate.trace_id,
            user_ids: &candidate.user_ids,
            agent_name: &candidate.agent_name,
            service_name: &candidate.service_name,
            cluster_id: &assignment.cluster_id,
            category: &assignment.label,
            start_ts: litellm_traces::iso_time(candidate.start_ms),
            computed_at: now,
            resolution_version: RESOLUTION_VERSION,
        },
    )
    .await?;
    Ok(true)
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

/// Assignments expire with their run's start like every trace table, so runs that are about to
/// expire are never sampled or classified; an assignment for one would vanish and be paid for again.
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
    Ok(fetch::<DiscoverySamples>(client, config.storage().reader(), &retained(config, params)).await?)
}

/// Unrecorded runs of the agent that this taxonomy version has not settled yet, newest first.
pub async fn bucketing_candidates(
    client: &Client,
    config: &Config,
    params: &BucketingParams,
) -> Result<Vec<BucketingCandidate>, Error> {
    Ok(fetch::<BucketingCandidates>(client, config.storage().reader(), &retained(config, params)).await?)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentOutcome {
    Assigned,
    Unknown,
    Failed,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct AssignmentRow<'a> {
    team_id: &'a str,
    api_key_hash: &'a str,
    trace_id: &'a str,
    user_ids: &'a [String],
    agent_name: &'a str,
    taxonomy_id: &'a str,
    taxonomy_version: u32,
    task_id: &'a str,
    outcome: AssignmentOutcome,
    attempts: u8,
    start_ts: String,
    computed_at: u64,
}

pub struct TaskAssignment<'a> {
    pub candidate: &'a BucketingCandidate,
    pub agent: &'a str,
    pub taxonomy_id: &'a str,
    pub taxonomy_version: u32,
    pub task_id: &'a str,
    pub outcome: AssignmentOutcome,
}

pub async fn record_assignment(
    client: &Client,
    config: &Config,
    assignment: &TaskAssignment<'_>,
) -> Result<(), Error> {
    let now = (OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as u64;
    insert(
        client,
        config,
        "lens_task_profile_assignments",
        &AssignmentRow {
            team_id: &assignment.candidate.team_id,
            api_key_hash: &assignment.candidate.api_key_hash,
            trace_id: &assignment.candidate.trace_id,
            user_ids: &assignment.candidate.user_ids,
            agent_name: assignment.agent,
            taxonomy_id: assignment.taxonomy_id,
            taxonomy_version: assignment.taxonomy_version,
            task_id: assignment.task_id,
            outcome: assignment.outcome,
            attempts: assignment.candidate.attempts.saturating_add(1),
            start_ts: litellm_traces::iso_time(assignment.candidate.start_ms),
            computed_at: now,
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{TaskInput, descriptor, ownership_hash};
    use rstest::rstest;

    #[rstest]
    #[case::ascii("x")]
    #[case::unicode("雪")]
    fn descriptor_bounds_bytes_without_splitting_unicode(#[case] character: &str) {
        let input = TaskInput {
            input: character.repeat(2048),
            tool: "shell".into(),
            recorded: String::new(),
        };
        let text = descriptor(&[input]);
        assert!(text.len() <= lens_profiles::MAX_TASK_BYTES);
        assert!(text.starts_with(character));
        assert!(!text.contains("shell"));
    }

    #[rstest]
    fn cluster_ownership_is_order_independent_without_mixing_users() {
        let first = ownership_hash(&["alice".into(), "bob".into()]);
        assert_eq!(
            first,
            ownership_hash(&["bob".into(), "alice".into(), "alice".into()])
        );
        assert_ne!(first, ownership_hash(&["alice".into()]));
        assert_ne!(
            ownership_hash(&["ab".into(), "c".into()]),
            ownership_hash(&["a".into(), "bc".into()])
        );
    }
}
