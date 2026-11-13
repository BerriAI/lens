mod evidence;
mod gateway;
mod http;
mod runtime;
mod store;

use chrono::Utc;
use lens_contract::{
    auth::{Identity, Role},
    optimizations::*,
};
use lens_datasets::DatasetRepository;
use litellm_storage_clickhouse::{
    datasets::Datasets,
    state::{Change, ClickHouseState, Snapshot},
};
use serde_json::json;
use std::sync::{Arc, RwLock};

use crate::error::OptimizationError as Error;
pub use http::router;
use store::{Record, Store, digest, key};

#[derive(Clone)]
pub struct Optimizations(Arc<Inner>);

struct Inner {
    store: Store,
    state: Arc<crate::State>,
    gateway: RwLock<Option<gateway::Gateway>>,
}

impl Optimizations {
    pub fn new(state: Arc<crate::State>, store: ClickHouseState) -> Self {
        Self(Arc::new(Inner {
            store: Store(store),
            state,
            gateway: RwLock::new(None),
        }))
    }

    pub fn configure(&self, models: &crate::gateway::Models) -> Result<(), Error> {
        let gateway = models
            .optimization_config()
            .map(|(config, identity)| gateway::Gateway::new(config, identity))
            .transpose()?;
        *self
            .0
            .gateway
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = gateway;
        Ok(())
    }

    fn gateway(&self) -> Result<gateway::Gateway, Error> {
        self.0
            .gateway
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .ok_or(Error::Gateway)
    }

    pub async fn models(&self) -> Result<Vec<gateway::Model>, Error> {
        self.gateway()?.models().await
    }

    async fn create(&self, config: OptimizationConfig) -> Result<OptimizationRun, Error> {
        config.validate().map_err(Error::Invalid)?;
        let catalog = self.models().await?;
        if std::iter::once(&config.baseline)
            .chain(config.models.iter())
            .chain((config.judge == OptimizationJudge::Llm).then_some(&config.judge_model))
            .any(|model| !catalog.iter().any(|entry| &entry.model_group == model))
        {
            return Err(Error::Invalid(
                "A selected model is not available through the configured gateway",
            ));
        }
        let registry = self.0.store.0.read("optimization-registry").await?;
        if self.0.store.records().await?.len() >= 100 {
            return Err(Error::Invalid("The optimization limit has been reached"));
        }
        let now = Utc::now().timestamp_millis();
        let record = Record {
            id: uuid::Uuid::new_v4().to_string(),
            config,
            status: OptimizationStatus::Running,
            created_at: now,
            error: None,
            scan_cursor: None,
            scan_until: now,
        };
        let mut changes = vec![
            Change {
                previous: Snapshot::empty(key(&record.id)?),
                value: serde_json::to_value(&record)?,
            },
            Change {
                previous: registry,
                value: json!({"last_created":record.id}),
            },
        ];
        if record.config.mode == OptimizationMode::Live {
            let config = &record.config;
            let previous = self
                .0
                .store
                .0
                .read(&format!(
                    "optimization-live/{}",
                    digest(&[&config.team_id, &config.agent, &config.baseline])
                ))
                .await?;
            if let Some(id) = previous.value.as_str()
                && let Ok((_, existing)) = self.0.store.get(id).await
                && existing.status != OptimizationStatus::Completed
            {
                return Err(Error::Invalid(
                    "Finish the existing Live optimization for this agent and baseline first",
                ));
            }
            changes.push(Change {
                previous,
                value: json!(record.id),
            });
        }
        if record.config.mode == OptimizationMode::Offline {
            let dataset = Datasets(self.0.store.0.clone())
                .get(&record.config.dataset_id, Some(record.config.revision))
                .await
                .map_err(|_| Error::Evidence)?
                .ok_or(Error::NotFound)?;
            if dataset.team_id != record.config.team_id
                || (!dataset.agent_name.is_empty() && dataset.agent_name != record.config.agent)
            {
                return Err(Error::Invalid(
                    "The dataset must belong to the selected agent and team",
                ));
            }
            let cases: Vec<_> = dataset.cases.iter().filter(|case| case.included).collect();
            if cases.is_empty() || cases.len() > 500 {
                return Err(Error::Invalid("Choose between one and 500 included cases"));
            }
            for case in cases {
                let source = self.offline_source(&record, case).await?;
                for attempt in source.attempts(&record) {
                    changes.push(Change {
                        previous: Snapshot::empty(format!(
                            "{}{}",
                            store::attempt_prefix(&record.id)?,
                            attempt.result.id
                        )),
                        value: serde_json::to_value(attempt)?,
                    });
                }
            }
        }
        self.0.store.0.commit(changes).await?;
        self.0.store.view(record).await
    }

    async fn status(&self, id: &str, status: OptimizationStatus) -> Result<OptimizationRun, Error> {
        let (snapshot, mut record) = self.0.store.get(id).await?;
        if record.status == OptimizationStatus::Completed && status != record.status {
            return Err(Error::Invalid(
                "Create a new optimization to rerun a completed experiment",
            ));
        }
        record.status = status;
        record.error = None;
        let mut changes = vec![Change {
            previous: snapshot,
            value: serde_json::to_value(&record)?,
        }];
        if status == OptimizationStatus::Completed {
            for (previous, mut attempt) in self.0.store.attempts(id).await? {
                if attempt.state != store::AttemptState::Complete {
                    attempt.state = store::AttemptState::Complete;
                    attempt.result.error =
                        Some("The comparison finished before this result was collected".into());
                    attempt.result.completed_at = Utc::now().timestamp_millis();
                    changes.push(Change {
                        previous,
                        value: serde_json::to_value(attempt)?,
                    });
                }
            }
        }
        self.0.store.0.commit(changes).await?;
        self.0.store.view(record).await
    }

    async fn assign(
        &self,
        identity: &Identity,
        request: OptimizationAssignmentRequest,
    ) -> Result<OptimizationAssignment, Error> {
        if request.trace_id.is_empty()
            || request.trace_id.len() > 128
            || request.agent.len() > 256
            || request.baseline.len() > 256
        {
            return Err(Error::Invalid("Invalid agent run identity"));
        }
        if matches!(
            identity.user_role,
            Role::ProxyAdminViewer | Role::InternalUserViewer | Role::Customer
        ) {
            return Err(Error::Forbidden);
        }
        let team = identity.team_id.as_deref().unwrap_or_default();
        if team.is_empty() && identity.user_role != Role::ProxyAdmin {
            return Err(Error::Forbidden);
        }
        let fallback = OptimizationAssignment {
            optimization_id: None,
            trace_id: request.trace_id.clone(),
            model: request.baseline.clone(),
            baseline: true,
        };
        let experiment = self
            .0
            .store
            .0
            .read(&format!(
                "optimization-live/{}",
                digest(&[team, &request.agent, &request.baseline])
            ))
            .await?;
        let Some(id) = experiment.value.as_str() else {
            return Ok(fallback);
        };
        let (_, record) = self.0.store.get(id).await?;
        if record.config.team_id != team || record.status != OptimizationStatus::Running {
            return Ok(fallback);
        }
        let assignment_key = format!(
            "optimization-assignment/{id}/{}",
            digest(&[&request.trace_id])
        );
        let snapshot = self.0.store.0.read(&assignment_key).await?;
        if !snapshot.value.is_null() {
            let assignment: OptimizationAssignment = serde_json::from_value(snapshot.value)?;
            return Ok(if eligible(identity, &assignment.model) {
                assignment
            } else {
                fallback
            });
        }
        let count = self
            .0
            .store
            .0
            .read(&format!("optimization-admission/{id}"))
            .await?;
        let admitted = count.value.as_u64().unwrap_or(0);
        if admitted >= 5000 {
            return Ok(fallback);
        }
        let candidates: Vec<_> = record
            .config
            .models
            .iter()
            .filter(|model| eligible(identity, model))
            .collect();
        if candidates.is_empty() {
            return Ok(fallback);
        }
        let sampled =
            bucket(&[id, &request.trace_id, "traffic"]) % 100 < u64::from(record.config.traffic);
        let model = if sampled {
            candidates
                [(bucket(&[id, &request.trace_id, "model"]) % candidates.len() as u64) as usize]
                .clone()
        } else {
            request.baseline.clone()
        };
        let assignment = OptimizationAssignment {
            optimization_id: Some(id.into()),
            trace_id: request.trace_id,
            baseline: model == request.baseline,
            model,
        };
        match self
            .0
            .store
            .0
            .commit(vec![
                Change {
                    previous: snapshot,
                    value: serde_json::to_value(&assignment)?,
                },
                Change {
                    previous: count,
                    value: json!(admitted + 1),
                },
            ])
            .await
        {
            Ok(_) => Ok(assignment),
            Err(litellm_storage_clickhouse::Error::StateConflict) => {
                let stored = self.0.store.0.read(&assignment_key).await?.value;
                if stored.is_null() {
                    return Ok(fallback);
                }
                let assignment: OptimizationAssignment = serde_json::from_value(stored)?;
                Ok(if eligible(identity, &assignment.model) {
                    assignment
                } else {
                    fallback
                })
            }
            Err(error) => Err(error.into()),
        }
    }
}

fn bucket(parts: &[&str]) -> u64 {
    u64::from_str_radix(&digest(parts)[..16], 16).unwrap_or(0)
}

fn eligible(identity: &Identity, model: &str) -> bool {
    identity.user_role == Role::ProxyAdmin
        || identity
            .models
            .iter()
            .any(|allowed| allowed == model || allowed == "all-proxy-models")
}
