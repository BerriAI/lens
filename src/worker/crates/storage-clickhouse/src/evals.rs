use chrono::{DateTime, Utc};
use lens_contract::eval::{CaseResult, CreateEvalRun, EvalRun, RunStatus};
use lens_evals::{Evaluation, RunError, RunRepository, ScoringLease, StoredRun, Submission};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    Error,
    state::{Change, ClickHouseState, backoff},
};

const KEY_ENCODING: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

#[derive(Clone)]
pub struct Evals(pub ClickHouseState);

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Idempotency {
    spec: CreateEvalRun,
    run_id: String,
}

fn failure(error: impl std::error::Error + Send + Sync + 'static) -> RunError {
    RunError::Unavailable(Box::new(error))
}

fn decode<T: DeserializeOwned>(value: serde_json::Value) -> Result<T, RunError> {
    let fields: serde_json::Map<String, serde_json::Value> =
        serde_json::from_value(value).map_err(failure)?;
    serde_json::from_value(serde_json::Value::Object(fields)).map_err(failure)
}

fn key(namespace: &str, identity: impl Serialize) -> Result<String, RunError> {
    let encoded = serde_json::to_string(&identity).map_err(failure)?;
    Ok(format!(
        "{namespace}/{}",
        utf8_percent_encode(&encoded, KEY_ENCODING)
    ))
}

fn owned(current: &StoredRun, expected: &StoredRun) -> Result<(), RunError> {
    if current.run.status != RunStatus::Scoring
        || current.version != expected.version
        || current.lease.is_none()
        || current.lease != expected.lease
    {
        return Err(RunError::StaleLease);
    }
    Ok(())
}

impl Evals {
    async fn update(
        &self,
        id: &str,
        transform: impl Fn(&StoredRun) -> Result<Option<StoredRun>, RunError> + Send + Sync,
    ) -> Result<Option<StoredRun>, RunError> {
        let key = key("eval-run", (id,))?;
        for attempt in 0..40 {
            let previous = self.0.read(&key).await.map_err(failure)?;
            if previous.value.is_null() {
                return Err(RunError::NotFound);
            }
            let current: StoredRun = decode(previous.value.clone())?;
            let Some(candidate) = transform(&current)? else {
                return Ok(None);
            };
            if candidate == current {
                return Ok(Some(current));
            }
            let updated = StoredRun {
                version: current.version.checked_add(1).ok_or(RunError::InvalidRun)?,
                ..candidate
            };
            let value = serde_json::to_value(&updated).map_err(failure)?;
            match self.0.commit(vec![Change { previous, value }]).await {
                Ok(()) => return Ok(Some(updated)),
                Err(Error::StateConflict) => (),
                Err(error) => return Err(failure(error)),
            }
            backoff(attempt).await;
        }
        Err(RunError::Conflict)
    }

    async fn terminate(
        &self,
        expected: &StoredRun,
        now: DateTime<Utc>,
        evaluation: Option<&Evaluation>,
        failure: &str,
    ) -> Result<(), RunError> {
        self.update(&expected.run.id, |current| {
            owned(current, expected)?;
            if !current
                .lease
                .as_ref()
                .is_some_and(|lease| lease.until > now)
            {
                return Err(RunError::StaleLease);
            }
            Ok(Some(StoredRun {
                run: EvalRun {
                    status: if evaluation.is_some() {
                        RunStatus::Done
                    } else {
                        RunStatus::Failed
                    },
                    summary: evaluation.map(|evaluation| evaluation.summary.clone()),
                    failure: failure.to_owned(),
                    ..current.run.clone()
                },
                completed_at: Some(now),
                verdicts: evaluation
                    .map(|evaluation| evaluation.verdicts.clone())
                    .unwrap_or_default(),
                resolved_traces: evaluation
                    .map(|evaluation| evaluation.resolved_traces.clone())
                    .unwrap_or_default(),
                lease: None,
                ..current.clone()
            }))
        })
        .await?;
        Ok(())
    }
}

impl RunRepository for Evals {
    async fn create(
        &self,
        idempotency_key: &str,
        candidate: &StoredRun,
    ) -> Result<StoredRun, RunError> {
        if idempotency_key.is_empty() || candidate.run.id.is_empty() {
            return Err(RunError::InvalidRun);
        }
        let run_key = key("eval-run", (&candidate.run.id,))?;
        let owner_key = key("eval-idempotency", (&candidate.team_id, idempotency_key))?;
        for attempt in 0..40 {
            let owner = self.0.read(&owner_key).await.map_err(failure)?;
            if !owner.value.is_null() {
                let owner: Idempotency = decode(owner.value)?;
                if owner.spec != candidate.spec {
                    return Err(RunError::IdempotencyConflict);
                }
                return self.get(&owner.run_id).await?.ok_or(RunError::InvalidRun);
            }
            let previous = self.0.read(&run_key).await.map_err(failure)?;
            if !previous.value.is_null() {
                return Err(RunError::Conflict);
            }
            let changes = vec![
                Change {
                    previous,
                    value: serde_json::to_value(candidate).map_err(failure)?,
                },
                Change {
                    previous: owner,
                    value: serde_json::to_value(Idempotency {
                        spec: candidate.spec.clone(),
                        run_id: candidate.run.id.clone(),
                    })
                    .map_err(failure)?,
                },
            ];
            match self.0.commit(changes).await {
                Ok(()) => return Ok(candidate.clone()),
                Err(Error::StateConflict) => (),
                Err(error) => return Err(failure(error)),
            }
            backoff(attempt).await;
        }
        Err(RunError::Conflict)
    }

    async fn get(&self, id: &str) -> Result<Option<StoredRun>, RunError> {
        let record = self
            .0
            .read(&key("eval-run", (id,))?)
            .await
            .map_err(failure)?;
        if record.value.is_null() {
            return Ok(None);
        }
        decode(record.value).map(Some)
    }

    async fn list(&self) -> Result<Vec<StoredRun>, RunError> {
        let mut after = String::new();
        let mut runs: Vec<StoredRun> = Vec::new();
        loop {
            let keys = self
                .0
                .keys("eval-run/", &after, 128)
                .await
                .map_err(failure)?;
            let Some(last) = keys.last() else { break };
            let references: Vec<_> = keys.iter().map(String::as_str).collect();
            for record in self.0.read_many(&references).await.map_err(failure)? {
                if !record.value.is_null() {
                    runs.push(decode(record.value)?);
                }
            }
            after = last.clone();
        }
        runs.sort_by(|left, right| left.run.id.cmp(&right.run.id));
        Ok(runs)
    }

    async fn submit(
        &self,
        id: &str,
        case_id: &str,
        trial: u32,
        result: &CaseResult,
        received_at: DateTime<Utc>,
    ) -> Result<(), RunError> {
        self.update(id, |current| {
            if current.run.status != RunStatus::Running {
                return Err(RunError::Closed);
            }
            if !current
                .cases
                .iter()
                .any(|case| case.id == case_id && case.included)
            {
                return Err(RunError::UnknownCase);
            }
            if trial >= current.spec.trials {
                return Err(RunError::InvalidTrial);
            }
            result.validate().map_err(|_| RunError::InvalidResult)?;
            let mut submissions = current.submissions.clone();
            if let Some(previous) = submissions
                .iter_mut()
                .find(|submission| submission.case_id == case_id && submission.trial == trial)
            {
                previous.result = result.clone();
            } else {
                submissions.push(Submission {
                    case_id: case_id.to_owned(),
                    trial,
                    result: result.clone(),
                    received_at,
                });
            }
            submissions.sort_by(|left, right| {
                (&left.case_id, left.trial).cmp(&(&right.case_id, right.trial))
            });
            Ok(Some(StoredRun {
                run: EvalRun {
                    received_trials: submissions.len() as u64,
                    ..current.run.clone()
                },
                submissions,
                ..current.clone()
            }))
        })
        .await?;
        Ok(())
    }

    async fn finish(&self, id: &str) -> Result<EvalRun, RunError> {
        self.update(id, |current| {
            if current.run.status != RunStatus::Running {
                return Err(RunError::Closed);
            }
            Ok(Some(StoredRun {
                run: EvalRun {
                    status: RunStatus::Scoring,
                    ..current.run.clone()
                },
                ..current.clone()
            }))
        })
        .await?
        .map(|stored| stored.run)
        .ok_or(RunError::InvalidRun)
    }

    async fn claim(
        &self,
        id: &str,
        owner: &str,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
    ) -> Result<Option<StoredRun>, RunError> {
        if owner.is_empty() || lease_until <= now {
            return Err(RunError::InvalidRun);
        }
        self.update(id, |current| {
            if current.run.status != RunStatus::Scoring
                || current
                    .lease
                    .as_ref()
                    .is_some_and(|lease| lease.until > now)
            {
                return Ok(None);
            }
            Ok(Some(StoredRun {
                lease: Some(ScoringLease {
                    owner: owner.to_owned(),
                    until: lease_until,
                }),
                ..current.clone()
            }))
        })
        .await
    }

    async fn complete(
        &self,
        expected: &StoredRun,
        evaluation: &Evaluation,
        now: DateTime<Utc>,
    ) -> Result<(), RunError> {
        self.terminate(expected, now, Some(evaluation), "").await
    }

    async fn renew(
        &self,
        expected: &StoredRun,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
    ) -> Result<StoredRun, RunError> {
        if lease_until <= now {
            return Err(RunError::InvalidRun);
        }
        self.update(&expected.run.id, |current| {
            owned(current, expected)?;
            let lease = current.lease.as_ref().ok_or(RunError::StaleLease)?;
            if lease.until <= now {
                return Err(RunError::StaleLease);
            }
            Ok(Some(StoredRun {
                lease: Some(ScoringLease {
                    until: lease.until.max(lease_until),
                    ..lease.clone()
                }),
                ..current.clone()
            }))
        })
        .await?
        .ok_or(RunError::StaleLease)
    }

    async fn fail(
        &self,
        expected: &StoredRun,
        failure: &str,
        now: DateTime<Utc>,
    ) -> Result<(), RunError> {
        self.terminate(expected, now, None, failure).await
    }

    async fn release(&self, expected: &StoredRun) -> Result<(), RunError> {
        self.update(&expected.run.id, |current| {
            owned(current, expected)?;
            Ok(Some(StoredRun {
                lease: None,
                ..current.clone()
            }))
        })
        .await?;
        Ok(())
    }
}
