use std::{sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use lens_contract::{
    profiles::{DiscoveryModelOptions, Taxonomy, TaxonomyStatus},
    worker::{ModelRequest, ModelRequestPrompt, ModelRequestPurpose},
};
use lens_server::profiles::taxonomy::{AgentState, Changes, claim, complete};
use litellm_storage_clickhouse::state::{Change, ClickHouseState};
use litellm_traces_clickhouse::profiles::{
    AssignmentOutcome, BucketingParams, TaskAssignment, bucketing_candidates, discovery_samples,
    record_assignment,
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

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct BucketLease {
    owner: String,
    version: u32,
    until: DateTime<Utc>,
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

/// The live taxonomy the worker may spend classification calls on: opted in, not a sample, and
/// with a window still open. Re-enabling alone never reopens one.
fn classifiable(state: &AgentState) -> Option<&Taxonomy> {
    state.active().filter(|active| {
        state.enabled()
            && !active.sample
            && active.windows.iter().any(|window| window.end_ms.is_none())
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

struct JobGuard<'a> {
    store: &'a ClickHouseState,
    taxonomy_id: &'a str,
    job_id: &'a str,
    attempt: u32,
}

impl JobGuard<'_> {
    async fn check(&self) -> Result<(), DiscoveryError> {
        match load_state(self.store, self.taxonomy_id).await {
            Ok((state, _)) if discovery_current(&state, self.job_id, self.attempt) => Ok(()),
            _ => Err(DiscoveryError::Cancelled),
        }
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
        Ok(gateway::embed(&self.models.client(), &connection, model, texts).await?)
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
            store: &self.store,
            taxonomy_id,
            job_id: &job.job_id,
            attempt: job.attempts,
        };
        let outcome = self
            .run(&claimed, &guard)
            .await
            .map_err(|error| error.to_string());
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

    async fn lease(&self, taxonomy: &Taxonomy) -> bool {
        let key = format!("{BUCKET_PREFIX}{}", taxonomy.taxonomy_id);
        let Ok(previous) = self.store.read(&key).await else {
            return false;
        };
        let current: Option<BucketLease> = serde_json::from_value(previous.value.clone()).ok();
        let Some(next) = take_lease(current.as_ref(), &self.owner, taxonomy.version, Utc::now())
        else {
            return false;
        };
        let Ok(value) = serde_json::to_value(next) else {
            return false;
        };
        self.store
            .commit(vec![Change { previous, value }])
            .await
            .is_ok()
    }

    async fn still_current(&self, snapshot: &Taxonomy) -> bool {
        matches!(
            load_state(&self.store, &snapshot.taxonomy_id).await,
            Ok((state, _)) if classification_current(&state, snapshot)
        )
    }

    /// Settles a bounded batch of the agent's unrecorded runs that start inside the taxonomy's
    /// classification windows.
    async fn bucket(&self, taxonomy_id: &str) -> bool {
        let Ok((state, _)) = load_state(&self.store, taxonomy_id).await else {
            return false;
        };
        let Some(taxonomy) = classifiable(&state).cloned() else {
            return false;
        };
        if !self.decision_supported(&taxonomy.models.decision) || !self.lease(&taxonomy).await {
            return false;
        }
        let windows = ranges(&taxonomy.windows, Utc::now().timestamp_millis() + SETTLE_MS);
        let Ok(candidates) = bucketing_candidates(
            &self.state.storage.client,
            &self.state.storage.config,
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
        for candidate in &candidates {
            if !self.still_current(&taxonomy).await || !self.lease(&taxonomy).await {
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
            let (outcome, task_id) = match &decided {
                Ok(Decision::Assigned(task_id)) => (AssignmentOutcome::Assigned, task_id.as_str()),
                Ok(Decision::Unknown) => (AssignmentOutcome::Unknown, ""),
                Err(_) => (AssignmentOutcome::Failed, ""),
            };
            let recorded = record_assignment(
                &self.state.storage.client,
                &self.state.storage.config,
                &TaskAssignment {
                    candidate,
                    agent: &taxonomy.agent,
                    taxonomy_id: &taxonomy.taxonomy_id,
                    taxonomy_version: taxonomy.version,
                    task_id,
                    outcome,
                },
            )
            .await;
            if recorded.is_err() {
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
    use rstest::rstest;

    fn taxonomy(status: TaxonomyStatus, windows: Vec<ClassificationWindow>) -> Taxonomy {
        Taxonomy {
            taxonomy_id: "tax".into(),
            agent: "agent".into(),
            version: 3,
            revision: 7,
            status,
            sample: false,
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
    #[case::opted_in_with_an_open_window(true, open(), false, true)]
    #[case::turned_off(false, open(), false, false)]
    #[case::turned_back_on_but_not_resumed(true, vec![ClassificationWindow { start_ms: 0, end_ms: Some(5) }], false, false)]
    #[case::sample(true, open(), true, false)]
    fn classification_spends_only_on_an_opted_in_open_live_taxonomy(
        #[case] enabled: bool,
        #[case] windows: Vec<ClassificationWindow>,
        #[case] sample: bool,
        #[case] expected: bool,
    ) {
        let active = Taxonomy {
            sample,
            ..taxonomy(TaxonomyStatus::Frozen, windows)
        };

        assert_eq!(
            classifiable(&state(enabled, None, Some(active))).is_some(),
            expected
        );
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
