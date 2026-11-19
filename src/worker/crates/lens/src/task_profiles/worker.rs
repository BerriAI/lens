use std::{sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use futures_util::future::BoxFuture;
use lens_contract::{
    profiles::{DiscoveryModelOptions, Taxonomy, TaxonomyStatus},
    worker::{ModelRequest, ModelRequestPrompt, ModelRequestPurpose},
};
use lens_server::profiles::{
    ProfileStoreError,
    taxonomy::{AgentState, Changes, claim, complete},
};
use litellm_storage_clickhouse::state::{Change, ClickHouseState};
use litellm_traces_clickhouse::profiles::{
    AssignmentOutcome, AssignmentRecord, BucketingParams, TaskAssignment, assignment_start,
    bucketing_candidates, discovery_samples, record_assignment,
};
use serde::{Deserialize, Serialize};

use super::{
    ACTIVE_PREFIX, TAXONOMY_PREFIX,
    bucketing::{self, DECISION_PROVIDERS, Decision},
    discovery::{self, Embeddings, Language, Sample},
    gateway::{self, InternalMetadata},
    load_state, ranges, save_state,
};
use crate::{State, error::DiscoveryError, gateway::Models};

const TICK: Duration = Duration::from_secs(5);
const IDLE: Duration = Duration::from_secs(30);
const BUCKET_BATCH: u32 = 20;
const KEY_PAGE: u32 = 100;
const BUCKET_ATTEMPTS: u8 = 3;
const DISCOVERY_BUDGET: Duration = Duration::from_secs(25 * 60);
const BUCKET_LEASE: chrono::Duration = chrono::Duration::minutes(5);
const BUCKET_PREFIX: &str = "profiles/bucketing/";
const SETTLE_MS: i64 = 60_000;

/// `pending` is the one decided assignment whose insert is not confirmed yet. It is queued before
/// the insert and survives takeovers and version changes, and whoever holds the lease next
/// replays it before anything else, so a stalled insert and its replay are the same row.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct BucketLease {
    owner: String,
    version: u32,
    until: DateTime<Utc>,
    #[serde(default)]
    pending: Option<AssignmentRecord>,
}

/// One replica buckets a taxonomy at a time; another may take over once the lease lapses.
fn take_lease(
    current: Option<&BucketLease>,
    owner: &str,
    version: u32,
    now: DateTime<Utc>,
) -> Option<BucketLease> {
    if current.is_some_and(|lease| lease.owner != owner && lease.until > now) {
        return None;
    }
    Some(BucketLease {
        owner: owner.to_owned(),
        version,
        until: now + BUCKET_LEASE,
        pending: current.and_then(|lease| lease.pending.clone()),
    })
}

fn held<'a>(
    current: Option<&'a BucketLease>,
    owner: &str,
    version: u32,
) -> Option<&'a BucketLease> {
    current.filter(|lease| lease.owner == owner && lease.version == version)
}

fn renew(
    current: Option<&BucketLease>,
    owner: &str,
    version: u32,
    now: DateTime<Utc>,
) -> Option<BucketLease> {
    let lease = held(current, owner, version)
        .filter(|lease| lease.pending.is_none())?;
    Some(BucketLease {
        until: now + BUCKET_LEASE,
        ..lease.clone()
    })
}

fn queue(
    current: Option<&BucketLease>,
    owner: &str,
    version: u32,
    record: &AssignmentRecord,
    now: DateTime<Utc>,
) -> Option<BucketLease> {
    renew(current, owner, version, now).map(|lease| BucketLease {
        pending: Some(record.clone()),
        ..lease
    })
}

fn clear(
    current: Option<&BucketLease>,
    owner: &str,
    version: u32,
    record: &AssignmentRecord,
) -> Option<BucketLease> {
    let lease = held(current, owner, version)
        .filter(|lease| lease.pending.as_ref() == Some(record))?;
    Some(BucketLease {
        pending: None,
        ..lease.clone()
    })
}

fn discovery_current(state: &AgentState, job_id: &str, attempt: u32) -> bool {
    state.enabled()
        && state.working.as_ref().is_some_and(|working| {
            working.status == TaxonomyStatus::Discovering
                && working
                    .job
                    .as_ref()
                    .is_some_and(|job| job.job_id == job_id && job.attempts == attempt)
        })
}

/// The live taxonomy the worker may spend classification calls on: opted in and with a window
/// still open. Re-enabling alone never reopens one.
fn classifiable(state: &AgentState) -> Option<&Taxonomy> {
    state.active().filter(|active| {
        state.enabled() && active.windows.iter().any(|window| window.end_ms.is_none())
    })
}

fn classification_current(state: &AgentState, snapshot: &Taxonomy) -> bool {
    classifiable(state).is_some_and(|active| {
        active.version == snapshot.version && active.revision == snapshot.revision
    })
}

pub(crate) fn model_options(models: &Models) -> DiscoveryModelOptions {
    let names = |groups: Vec<lens_contract::activity::AnalysisModelInfo>| {
        groups.into_iter().map(|group| group.model_group).collect()
    };
    DiscoveryModelOptions {
        discovery: names(models.analysis().model_groups()),
        embedding: models.embedding_models(),
        classification: names(
            models
                .evaluation()
                .model_groups()
                .into_iter()
                .filter(|group| {
                    !group.providers.is_empty()
                        && group
                            .providers
                            .iter()
                            .all(|provider| DECISION_PROVIDERS.contains(&provider.as_str()))
                })
                .collect(),
        ),
    }
}

#[derive(Clone)]
pub struct ProfilesWorker {
    state: Arc<State>,
    store: ClickHouseState,
    models: Models,
    owner: String,
}

/// Where a running discovery reads whether its job is still current.
trait StateSource: Send + Sync {
    fn load<'s>(
        &'s self,
        taxonomy_id: &'s str,
    ) -> BoxFuture<'s, Result<AgentState, ProfileStoreError>>;
}

impl StateSource for ClickHouseState {
    fn load<'s>(
        &'s self,
        taxonomy_id: &'s str,
    ) -> BoxFuture<'s, Result<AgentState, ProfileStoreError>> {
        Box::pin(async move { load_state(self, taxonomy_id).await.map(|(state, _)| state) })
    }
}

/// Checked before every paid stage. Unreadable state stops the run without calling it cancelled.
struct JobGuard<'a> {
    source: &'a dyn StateSource,
    taxonomy_id: &'a str,
    job_id: &'a str,
    attempt: u32,
}

impl JobGuard<'_> {
    async fn check(&self) -> Result<(), DiscoveryError> {
        match self.source.load(self.taxonomy_id).await {
            Ok(state) if discovery_current(&state, self.job_id, self.attempt) => Ok(()),
            Ok(_) => Err(DiscoveryError::Cancelled),
            Err(_) => Err(DiscoveryError::StateUnavailable),
        }
    }
}

/// A run stopped by unreadable state keeps its lease, so the job is retried once the lease ends
/// instead of being marked failed.
fn recorded<T>(outcome: Result<T, DiscoveryError>) -> Option<Result<T, String>> {
    match outcome {
        Err(DiscoveryError::StateUnavailable) => None,
        other => Some(other.map_err(|error| error.to_string())),
    }
}

struct GatewayLanguage<'a> {
    models: &'a Models,
    guard: &'a JobGuard<'a>,
}

impl Language for GatewayLanguage<'_> {
    async fn complete(&self, model: &str, prompt: String) -> Result<String, DiscoveryError> {
        self.guard.check().await?;
        let analysis = self.models.analysis();
        let prepared = analysis
            .prepare(
                model,
                &ModelRequest {
                    messages: Vec::new(),
                    prompt: ModelRequestPrompt::try_from(prompt.as_str())
                        .map_err(|_| DiscoveryError::InvalidReply("prompt"))?,
                    purpose: ModelRequestPurpose::Cluster,
                },
            )
            .await
            .map_err(|_| DiscoveryError::Unconfigured("language"))?;
        if prepared.context_exceeded {
            return Err(DiscoveryError::Provider("language"));
        }
        analysis
            .complete_with_gateway_metadata(&prepared, &InternalMetadata::new())
            .await
            .map(|completion| completion.result.content)
            .map_err(|_| DiscoveryError::Provider("language"))
    }
}

struct GatewayEmbeddings<'a> {
    models: &'a Models,
    guard: &'a JobGuard<'a>,
}

impl Embeddings for GatewayEmbeddings<'_> {
    async fn embed(&self, model: &str, texts: &[String]) -> Result<Vec<Vec<f32>>, DiscoveryError> {
        self.guard.check().await?;
        let connection = self
            .models
            .connection()
            .ok_or(DiscoveryError::Unconfigured("embedding"))?;
        let signing = self.models.signing();
        Ok(gateway::embed(
            &self.models.client(),
            &connection,
            signing.as_ref(),
            model,
            texts,
        )
        .await?)
    }
}

impl ProfilesWorker {
    pub fn new(state: Arc<State>, store: ClickHouseState, models: Models) -> Self {
        Self {
            state,
            store,
            models,
            owner: uuid::Uuid::new_v4().to_string(),
        }
    }

    pub async fn serve(self) {
        loop {
            let worked = match self.tick().await {
                Ok(worked) => worked,
                Err(error) => {
                    tracing::warn!(%error, "task profile worker tick failed");
                    false
                }
            };
            tokio::time::sleep(if worked { TICK } else { IDLE }).await;
        }
    }

    async fn ids(&self, prefix: &str) -> Result<Vec<String>, litellm_storage_clickhouse::Error> {
        let mut found = Vec::new();
        let mut after = String::new();
        loop {
            let keys = self.store.keys(prefix, &after, KEY_PAGE).await?;
            let Some(last) = keys.last().cloned() else {
                return Ok(found);
            };
            found.extend(
                keys.iter()
                    .filter_map(|key| key.strip_prefix(prefix))
                    .map(str::to_owned),
            );
            after = last;
        }
    }

    async fn tick(&self) -> Result<bool, litellm_storage_clickhouse::Error> {
        if self.state.require_storage().is_err() {
            return Ok(false);
        }
        let mut worked = false;
        for id in self.ids(TAXONOMY_PREFIX).await? {
            worked |= self.discover(&id).await;
        }
        for id in self.ids(ACTIVE_PREFIX).await? {
            worked |= self.bucket(&id).await;
        }
        Ok(worked)
    }

    async fn discover(&self, taxonomy_id: &str) -> bool {
        let Ok((state, heads)) = load_state(&self.store, taxonomy_id).await else {
            return false;
        };
        let Some(current) = state.working.as_ref().filter(|_| state.enabled()) else {
            return false;
        };
        let Some(claimed) = claim(current, Utc::now()) else {
            return false;
        };
        let changes = Changes {
            working: Some(claimed.clone()),
            ..Changes::default()
        };
        if save_state(&self.store, heads, changes).await.is_err() {
            return false;
        }
        let Some(job) = claimed.job.clone() else {
            return true;
        };
        let guard = JobGuard {
            source: &self.store,
            taxonomy_id,
            job_id: &job.job_id,
            attempt: job.attempts,
        };
        let Some(outcome) = recorded(self.run(&claimed, &guard).await) else {
            return true;
        };
        let Ok((latest, heads)) = load_state(&self.store, taxonomy_id).await else {
            return true;
        };
        if !discovery_current(&latest, &job.job_id, job.attempts) {
            return true;
        }
        if let Ok(finished) = complete(
            latest.working,
            &job.job_id,
            job.attempts,
            outcome,
            Utc::now(),
        ) {
            let changes = Changes {
                working: Some(finished),
                ..Changes::default()
            };
            let _ = save_state(&self.store, heads, changes).await;
        }
        true
    }

    fn decision_supported(&self, model: &str) -> bool {
        model_options(&self.models)
            .classification
            .iter()
            .any(|alias| alias == model)
    }

    async fn run(
        &self,
        taxonomy: &Taxonomy,
        guard: &JobGuard<'_>,
    ) -> Result<
        (
            Vec<lens_contract::profiles::TaskDefinition>,
            Vec<lens_contract::profiles::DiscoveryPoint>,
        ),
        DiscoveryError,
    > {
        if !self.decision_supported(&taxonomy.models.decision) {
            return Err(DiscoveryError::Unconfigured("Jev decision"));
        }
        let job = taxonomy
            .job
            .as_ref()
            .ok_or(DiscoveryError::InvalidReply("job"))?;
        let samples: Vec<Sample> = discovery_samples(
            &self.state.storage.client,
            &self.state.storage.config,
            &BucketingParams::windows(&taxonomy.agent, &[(job.start_ms, job.end_ms)], job.max_runs),
        )
        .await
        .map_err(|_| DiscoveryError::Provider("storage"))?
        .into_iter()
        .map(|candidate| Sample {
            team_id: candidate.team_id,
            api_key_hash: candidate.api_key_hash,
            trace_id: candidate.trace_id,
            text: candidate.input_preview,
        })
        .collect();
        if samples.len() < lens_profiles::clustering::MIN_POINTS {
            return Err(DiscoveryError::TooFewRuns);
        }
        discovery::discover_within(
            DISCOVERY_BUDGET,
            &GatewayLanguage {
                models: &self.models,
                guard,
            },
            &GatewayEmbeddings {
                models: &self.models,
                guard,
            },
            &taxonomy.models,
            &samples,
            || uuid::Uuid::new_v4().to_string(),
        )
        .await
    }

    async fn update_lease(
        &self,
        taxonomy_id: &str,
        next: impl FnOnce(Option<&BucketLease>) -> Option<BucketLease>,
    ) -> Option<BucketLease> {
        let key = format!("{BUCKET_PREFIX}{taxonomy_id}");
        let previous = self.store.read(&key).await.ok()?;
        let current: Option<BucketLease> = serde_json::from_value(previous.value.clone()).ok();
        let lease = next(current.as_ref())?;
        let value = serde_json::to_value(&lease).ok()?;
        self.store
            .commit(vec![Change { previous, value }])
            .await
            .ok()
            .map(|_| lease)
    }

    async fn store_assignment(
        &self,
        taxonomy_id: &str,
        version: u32,
        record: &AssignmentRecord,
    ) -> bool {
        let client = &self.state.storage.client;
        if record_assignment(client, &self.state.storage.config, record)
            .await
            .is_err()
        {
            return false;
        }
        self.update_lease(taxonomy_id, |current| {
            clear(current, &self.owner, version, record)
        })
        .await
        .is_some()
    }

    async fn still_current(&self, snapshot: &Taxonomy) -> bool {
        matches!(
            load_state(&self.store, &snapshot.taxonomy_id).await,
            Ok((state, _)) if classification_current(&state, snapshot)
        )
    }

    /// Replays a pending assignment before anything else, also while classification is paused,
    /// since its model call was already made.
    async fn bucket(&self, taxonomy_id: &str) -> bool {
        let Ok((state, _)) = load_state(&self.store, taxonomy_id).await else {
            return false;
        };
        let Some(version) = state.active.as_ref().map(|active| active.version) else {
            return false;
        };
        let taxonomy = classifiable(&state)
            .filter(|taxonomy| self.decision_supported(&taxonomy.models.decision))
            .cloned();
        let Some(lease) = self
            .update_lease(taxonomy_id, |current| {
                let pending = current.is_some_and(|lease| lease.pending.is_some());
                if !pending && taxonomy.is_none() {
                    return None;
                }
                take_lease(current, &self.owner, version, Utc::now())
            })
            .await
        else {
            return false;
        };
        if let Some(pending) = &lease.pending {
            if !pending.insertable() {
                return self
                    .update_lease(taxonomy_id, |current| {
                        clear(current, &self.owner, version, pending)
                    })
                    .await
                    .is_some();
            }
            return self.store_assignment(taxonomy_id, version, pending).await;
        }
        let Some(taxonomy) = taxonomy else {
            return false;
        };
        let windows = ranges(&taxonomy.windows, Utc::now().timestamp_millis() + SETTLE_MS);
        let client = &self.state.storage.client;
        let config = &self.state.storage.config;
        let Ok(candidates) = bucketing_candidates(
            client,
            config,
            &BucketingParams {
                taxonomy_id: taxonomy.taxonomy_id.clone(),
                taxonomy_version: taxonomy.version,
                require_input: 0,
                max_attempts: BUCKET_ATTEMPTS,
                ..BucketingParams::windows(&taxonomy.agent, &windows, BUCKET_BATCH)
            },
        )
        .await
        else {
            return false;
        };
        let evaluation = self.models.evaluation();
        let longest_task = taxonomy
            .tasks
            .iter()
            .map(|task| task.task_id.as_str())
            .max_by_key(|task_id| task_id.len())
            .unwrap_or_default();
        for candidate in &candidates {
            let largest = TaskAssignment {
                candidate,
                agent: &taxonomy.agent,
                taxonomy_id: &taxonomy.taxonomy_id,
                taxonomy_version: taxonomy.version,
                task_id: longest_task,
                outcome: AssignmentOutcome::Assigned,
            };
            if !largest.record(candidate.start_ms, u64::MAX).insertable() {
                continue;
            }
            let renewed = self
                .update_lease(taxonomy_id, |current| {
                    renew(current, &self.owner, version, Utc::now())
                })
                .await;
            if !self.still_current(&taxonomy).await || renewed.is_none() {
                return true;
            }
            let decided = bucketing::decide(
                evaluation.as_ref(),
                &taxonomy.models.decision,
                &taxonomy,
                &candidate.input_preview,
            )
            .await;
            if !self.still_current(&taxonomy).await {
                return true;
            }
            let Ok(stored) = assignment_start(client, config, candidate).await else {
                return true;
            };
            let (outcome, task_id) = match &decided {
                Ok(Decision::Assigned(task_id)) => (AssignmentOutcome::Assigned, task_id.as_str()),
                Ok(Decision::Unknown) => (AssignmentOutcome::Unknown, ""),
                Err(_) => (AssignmentOutcome::Failed, ""),
            };
            let assignment = TaskAssignment {
                candidate,
                agent: &taxonomy.agent,
                taxonomy_id: &taxonomy.taxonomy_id,
                taxonomy_version: taxonomy.version,
                task_id,
                outcome,
            };
            let start_ms = stored.unwrap_or(candidate.start_ms);
            let computed_at = Utc::now().timestamp_millis().unsigned_abs();
            let record = assignment.record(start_ms, computed_at);
            if !record.insertable() {
                continue;
            }
            let queued = self
                .update_lease(taxonomy_id, |current| {
                    queue(current, &self.owner, version, &record, Utc::now())
                })
                .await;
            if queued.is_none() || !self.store_assignment(taxonomy_id, version, &record).await {
                return true;
            }
        }
        !candidates.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lens_contract::profiles::{
        ClassificationWindow, DiscoveryJob, DiscoveryModelChoice, ProfileSettings, TaxonomyModels,
    };
    use litellm_traces_clickhouse::profiles::BucketingCandidate;
    use rstest::rstest;

    fn taxonomy(status: TaxonomyStatus, windows: Vec<ClassificationWindow>) -> Taxonomy {
        Taxonomy {
            taxonomy_id: "tax".into(),
            agent: "agent".into(),
            version: 3,
            revision: 7,
            status,
            tasks: Vec::new(),
            points: Vec::new(),
            job: Some(DiscoveryJob {
                job_id: "job".into(),
                requested_at: now(),
                lease_until: None,
                attempts: 1,
                max_runs: 100,
                start_ms: 0,
                end_ms: 1,
            }),
            discovered_at: None,
            frozen_at: None,
            error: None,
            models: TaxonomyModels::default(),
            discovery: None,
            windows,
        }
    }

    fn open() -> Vec<ClassificationWindow> {
        vec![ClassificationWindow {
            start_ms: 0,
            end_ms: None,
        }]
    }

    fn state(enabled: bool, working: Option<Taxonomy>, active: Option<Taxonomy>) -> AgentState {
        AgentState {
            working,
            active,
            settings: Some(ProfileSettings {
                agent: "agent".into(),
                enabled,
                models: DiscoveryModelChoice::default(),
                revision: 1,
                updated_at: None,
            }),
        }
    }

    #[rstest]
    #[case::opted_in_with_an_open_window(true, open(), true)]
    #[case::turned_off(false, open(), false)]
    #[case::turned_back_on_but_not_resumed(true, vec![ClassificationWindow { start_ms: 0, end_ms: Some(5) }], false)]
    fn classification_spends_only_on_an_opted_in_open_taxonomy(
        #[case] enabled: bool,
        #[case] windows: Vec<ClassificationWindow>,
        #[case] expected: bool,
    ) {
        let active = taxonomy(TaxonomyStatus::Frozen, windows);

        assert_eq!(
            classifiable(&state(enabled, None, Some(active))).is_some(),
            expected
        );
    }

    struct Stored(Option<AgentState>);

    impl StateSource for Stored {
        fn load<'s>(&'s self, _: &'s str) -> BoxFuture<'s, Result<AgentState, ProfileStoreError>> {
            Box::pin(async { self.0.clone().ok_or(ProfileStoreError::Unavailable) })
        }
    }

    #[rstest]
    #[case::still_running(Some(state(true, Some(taxonomy(TaxonomyStatus::Discovering, Vec::new())), None)), Ok(()))]
    #[case::turned_off(
        Some(state(false, Some(taxonomy(TaxonomyStatus::Discovering, Vec::new())), None)),
        Err(DiscoveryError::Cancelled)
    )]
    #[case::replaced(Some(state(true, None, None)), Err(DiscoveryError::Cancelled))]
    #[case::unreadable(None, Err(DiscoveryError::StateUnavailable))]
    #[tokio::test]
    async fn paid_stages_stop_unless_the_job_is_still_current(
        #[case] loaded: Option<AgentState>,
        #[case] expected: Result<(), DiscoveryError>,
    ) {
        let stored = Stored(loaded);
        let guard = JobGuard {
            source: &stored,
            taxonomy_id: "tax",
            job_id: "job",
            attempt: 1,
        };

        assert_eq!(guard.check().await, expected);
    }

    #[rstest]
    #[case::unreadable_state(Err(DiscoveryError::StateUnavailable), None)]
    #[case::cancelled(Err(DiscoveryError::Cancelled), Some(Err("discovery was cancelled or replaced".to_owned())))]
    #[case::finished(Ok(()), Some(Ok(())))]
    fn only_unreadable_state_leaves_the_job_for_a_retry(
        #[case] outcome: Result<(), DiscoveryError>,
        #[case] expected: Option<Result<(), String>>,
    ) {
        assert_eq!(recorded(outcome), expected);
    }

    #[rstest]
    fn a_decision_is_dropped_once_the_active_taxonomy_changes_under_it() {
        let snapshot = taxonomy(TaxonomyStatus::Frozen, open());
        let paused_and_resumed = Taxonomy {
            revision: snapshot.revision + 2,
            ..snapshot.clone()
        };

        assert!(classification_current(
            &state(true, None, Some(snapshot.clone())),
            &snapshot
        ));
        assert!(!classification_current(
            &state(true, None, Some(paused_and_resumed)),
            &snapshot
        ));
        assert!(!classification_current(
            &state(false, None, Some(snapshot.clone())),
            &snapshot
        ));
    }

    #[rstest]
    #[case::current(true, "job", 1, true)]
    #[case::turned_off(false, "job", 1, false)]
    #[case::replaced_job(true, "other", 1, false)]
    #[case::reclaimed_attempt(true, "job", 2, false)]
    fn discovery_calls_continue_only_for_the_claimed_attempt(
        #[case] enabled: bool,
        #[case] job_id: &str,
        #[case] attempt: u32,
        #[case] expected: bool,
    ) {
        let working = taxonomy(TaxonomyStatus::Discovering, Vec::new());

        assert_eq!(
            discovery_current(&state(enabled, Some(working), None), job_id, attempt),
            expected
        );
    }

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000, 0).expect("time")
    }

    #[rstest]
    fn a_second_replica_waits_while_the_first_holds_the_lease() {
        let first = take_lease(None, "replica-a", 2, now()).expect("first");

        let second = take_lease(Some(&first), "replica-b", 2, now());
        let renewed = take_lease(Some(&first), "replica-a", 2, now());

        assert_eq!(second, None);
        assert_eq!(
            renewed.map(|lease| lease.owner),
            Some("replica-a".to_owned())
        );
    }

    const FEBRUARY_MS: i64 = 1_769_990_400_000;
    const LATE_JANUARY_MS: i64 = 1_769_731_200_000;

    fn decided(version: u32, start_ms: i64) -> AssignmentRecord {
        TaskAssignment {
            candidate: &BucketingCandidate {
                team_id: "team".into(),
                api_key_hash: "key".into(),
                trace_id: "run".into(),
                user_ids: vec!["user".into()],
                start_ms: LATE_JANUARY_MS,
                input_preview: "private request text".into(),
                attempts: 0,
            },
            agent: "agent",
            taxonomy_id: "tax",
            taxonomy_version: version,
            task_id: "release",
            outcome: AssignmentOutcome::Assigned,
        }
        .record(start_ms, 7)
    }

    fn queued(owner: &str, record: &AssignmentRecord) -> BucketLease {
        let version = record.taxonomy_version;
        let lease = take_lease(None, owner, version, now());
        let queued = queue(lease.as_ref(), owner, version, record, now());
        queued.expect("queued")
    }

    fn lapsed() -> DateTime<Utc> {
        now() + BUCKET_LEASE + chrono::Duration::seconds(1)
    }

    #[rstest]
    fn a_takeover_replays_the_exact_stalled_assignment_before_new_work() {
        let record = decided(2, FEBRUARY_MS);
        let stalled = queued("replica-a", &record);

        let early = take_lease(Some(&stalled), "replica-b", 2, now());
        let taken = take_lease(Some(&stalled), "replica-b", 2, lapsed());
        let taken = taken.expect("taken");
        let new_work = renew(Some(&taken), "replica-b", 2, lapsed());
        let other = queue(
            Some(&taken),
            "replica-b",
            2,
            &decided(2, LATE_JANUARY_MS),
            lapsed(),
        );
        let late_stalled_holder = clear(Some(&taken), "replica-a", 2, &record);

        assert_eq!(early, None);
        assert_eq!(taken.pending.as_ref(), Some(&record));
        assert_eq!(new_work, None);
        assert_eq!(other, None);
        assert_eq!(late_stalled_holder, None);
    }

    #[rstest]
    fn a_confirmed_insert_clears_only_its_own_payload() {
        let record = decided(2, FEBRUARY_MS);
        let lease = queued("replica-a", &record);

        let mismatched = clear(Some(&lease), "replica-a", 2, &decided(2, LATE_JANUARY_MS));
        let cleared = clear(Some(&lease), "replica-a", 2, &record);
        let cleared = cleared.expect("cleared");

        assert_eq!(mismatched, None);
        assert_eq!(cleared.pending, None);
        assert!(renew(Some(&cleared), "replica-a", 2, now()).is_some());
    }

    #[rstest]
    #[case::same_replica_retries("replica-a", 2)]
    #[case::another_replica_takes_over("replica-b", 2)]
    #[case::a_new_version_is_activated("replica-a", 3)]
    fn an_unconfirmed_insert_keeps_its_month_and_version_until_replayed(
        #[case] owner: &str,
        #[case] version: u32,
    ) {
        let record = decided(2, FEBRUARY_MS);
        let failed = queued("replica-a", &record);

        let next = take_lease(Some(&failed), owner, version, lapsed());
        let next = next.expect("lease");
        let pending = next.pending.clone().expect("still pending");

        assert_eq!(pending, record);
        assert_eq!(pending.start_ts, litellm_traces::iso_time(FEBRUARY_MS));
        assert_eq!(pending.taxonomy_version, 2);
        assert_eq!(renew(Some(&next), owner, version, lapsed()), None);
        assert!(clear(Some(&next), owner, version, &record).is_some());
    }

    #[rstest]
    fn the_queued_payload_carries_no_request_text() {
        let lease = queued("replica-a", &decided(2, FEBRUARY_MS));

        let stored = serde_json::to_string(&lease).expect("lease json");

        assert!(!stored.contains("private request text"));
        assert_eq!(
            serde_json::from_str::<BucketLease>(&stored).ok(),
            Some(lease)
        );
    }

    #[rstest]
    fn a_lapsed_lease_can_be_taken_over() {
        let first = take_lease(None, "replica-a", 2, now()).expect("first");

        let taken = take_lease(
            Some(&first),
            "replica-b",
            2,
            now() + BUCKET_LEASE + chrono::Duration::seconds(1),
        );

        assert_eq!(taken.map(|lease| lease.owner), Some("replica-b".to_owned()));
    }
}
