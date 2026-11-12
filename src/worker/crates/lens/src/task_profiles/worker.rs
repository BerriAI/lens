use std::{sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use lens_contract::{
    profiles::{Taxonomy, TaxonomyStatus},
    worker::{ModelRequest, ModelRequestPrompt, ModelRequestPurpose},
};
use lens_server::profiles::taxonomy::{claim, complete};
use litellm_storage_clickhouse::state::{Change, ClickHouseState};
use litellm_traces_clickhouse::profiles::{
    AssignmentOutcome, BucketingParams, TaskAssignment, bucketing_candidates, discovery_samples,
    record_assignment,
};
use serde::{Deserialize, Serialize};

use super::{
    TAXONOMY_PREFIX,
    bucketing::{self, DECISION_PROVIDERS, Decision},
    decode,
    discovery::{self, Embeddings, Language, Sample},
    gateway::{self, InternalMetadata},
    read_taxonomy, write_taxonomy,
};
use crate::{State, error::DiscoveryError, gateway::Models};

const TICK: Duration = Duration::from_secs(5);
const IDLE: Duration = Duration::from_secs(30);
const BUCKET_BATCH: u32 = 20;
const BUCKET_LOOKBACK_MS: i64 = 30 * 86_400_000;
const TAXONOMY_PAGE: u32 = 100;
const BUCKET_ATTEMPTS: u8 = 3;
const DISCOVERY_BUDGET: Duration = Duration::from_secs(25 * 60);
const BUCKET_LEASE: chrono::Duration = chrono::Duration::minutes(5);
const BUCKET_PREFIX: &str = "profiles/bucketing/";

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

#[derive(Clone)]
pub struct ProfilesWorker {
    state: Arc<State>,
    store: ClickHouseState,
    models: Models,
    owner: String,
}

struct GatewayLanguage<'a>(&'a Models);

impl Language for GatewayLanguage<'_> {
    async fn complete(&self, model: &str, prompt: String) -> Result<String, DiscoveryError> {
        let analysis = self.0.analysis();
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

struct GatewayEmbeddings<'a>(&'a Models);

impl Embeddings for GatewayEmbeddings<'_> {
    async fn embed(&self, model: &str, texts: &[String]) -> Result<Vec<Vec<f32>>, DiscoveryError> {
        let connection = self
            .0
            .connection()
            .ok_or(DiscoveryError::Unconfigured("embedding"))?;
        Ok(gateway::embed(&self.0.client(), &connection, model, texts).await?)
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

    async fn taxonomies(&self) -> Result<Vec<Taxonomy>, litellm_storage_clickhouse::Error> {
        let mut found = Vec::new();
        let mut after = String::new();
        loop {
            let keys = self
                .store
                .keys(TAXONOMY_PREFIX, &after, TAXONOMY_PAGE)
                .await?;
            let Some(last) = keys.last().cloned() else {
                return Ok(found);
            };
            let references: Vec<&str> = keys.iter().map(String::as_str).collect();
            found.extend(
                self.store
                    .read_many(&references)
                    .await?
                    .iter()
                    .filter_map(|snapshot| decode(snapshot).ok().flatten()),
            );
            after = last;
        }
    }

    async fn tick(&self) -> Result<bool, litellm_storage_clickhouse::Error> {
        if self.state.require_storage().is_err() {
            return Ok(false);
        }
        let mut worked = false;
        for taxonomy in self.taxonomies().await? {
            worked |= match taxonomy.status {
                TaxonomyStatus::Discovering => self.discover(&taxonomy.taxonomy_id).await,
                TaxonomyStatus::Frozen if !taxonomy.sample => self.bucket(&taxonomy).await,
                _ => false,
            };
        }
        Ok(worked)
    }

    async fn discover(&self, taxonomy_id: &str) -> bool {
        let Ok((Some(current), head)) = read_taxonomy(&self.store, taxonomy_id).await else {
            return false;
        };
        let Some(claimed) = claim(&current, Utc::now()) else {
            return false;
        };
        if write_taxonomy(&self.store, head, Some(&claimed))
            .await
            .is_err()
        {
            return false;
        }
        let Some(job) = claimed.job.clone() else {
            return true;
        };
        let outcome = self.run(&claimed).await.map_err(|error| error.to_string());
        let Ok((latest, head)) = read_taxonomy(&self.store, taxonomy_id).await else {
            return true;
        };
        if let Ok(finished) = complete(latest, &job.job_id, job.attempts, outcome, Utc::now()) {
            let _ = write_taxonomy(&self.store, head, Some(&finished)).await;
        }
        true
    }

    fn decision_supported(&self, model: &str) -> bool {
        self.models
            .evaluation()
            .model_groups()
            .iter()
            .find(|group| group.model_group == model)
            .is_some_and(|group| {
                !group.providers.is_empty()
                    && group
                        .providers
                        .iter()
                        .all(|provider| DECISION_PROVIDERS.contains(&provider.as_str()))
            })
    }

    async fn run(
        &self,
        taxonomy: &Taxonomy,
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
            &BucketingParams {
                agent: taxonomy.agent.clone(),
                taxonomy_id: String::new(),
                taxonomy_version: 0,
                start_ms: job.start_ms,
                end_ms: job.end_ms,
                max_attempts: BUCKET_ATTEMPTS,
                limit: job.max_runs,
            },
        )
        .await
        .map_err(|_| DiscoveryError::Provider("storage"))?
        .into_iter()
        .filter(|candidate| !candidate.input_preview.trim().is_empty())
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
            &GatewayLanguage(&self.models),
            &GatewayEmbeddings(&self.models),
            &taxonomy.models,
            &samples,
            || uuid::Uuid::new_v4().to_string(),
        )
        .await
    }

    /// Settles a bounded batch of the agent's unrecorded runs against the frozen taxonomy: the
    /// backlog from before the freeze, including the discovery sample, as well as new arrivals.
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

    async fn still_frozen(&self, taxonomy: &Taxonomy) -> bool {
        matches!(
            read_taxonomy(&self.store, &taxonomy.taxonomy_id).await,
            Ok((Some(current), _)) if current.status == TaxonomyStatus::Frozen && current.version == taxonomy.version
        )
    }

    async fn bucket(&self, taxonomy: &Taxonomy) -> bool {
        if !self.decision_supported(&taxonomy.models.decision) || !self.lease(taxonomy).await {
            return false;
        }
        let now = Utc::now().timestamp_millis();
        let Ok(candidates) = bucketing_candidates(
            &self.state.storage.client,
            &self.state.storage.config,
            &BucketingParams {
                agent: taxonomy.agent.clone(),
                taxonomy_id: taxonomy.taxonomy_id.clone(),
                taxonomy_version: taxonomy.version,
                start_ms: now - BUCKET_LOOKBACK_MS,
                end_ms: now,
                max_attempts: BUCKET_ATTEMPTS,
                limit: BUCKET_BATCH,
            },
        )
        .await
        else {
            return false;
        };
        let evaluation = self.models.evaluation();
        for candidate in &candidates {
            if !self.still_frozen(taxonomy).await || !self.lease(taxonomy).await {
                return true;
            }
            let decided = bucketing::decide(
                evaluation.as_ref(),
                &taxonomy.models.decision,
                taxonomy,
                &candidate.input_preview,
            )
            .await;
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
    use rstest::rstest;

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
