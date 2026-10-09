use chrono::{DateTime, Utc};
use lens_contract::eval::RunStatus;

use super::records::{
    RunReference, baseline_prefix, descending_time, encode, retry, run_key, scoring_key, stored_run,
};
use super::{ATTEMPTS, EvalStore, RunCompletion, SCORING_LEASE_SECONDS, ScoringLease, StoredRun};
use crate::{
    Error, EvalError,
    state::{Change, Snapshot},
};

impl EvalStore {
    pub async fn claim_scoring(
        &self,
        team: &str,
        id: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<ScoringLease>, EvalError> {
        self.update_lease(team, id, LeaseUpdate::Claim(now)).await
    }

    pub async fn renew_scoring(
        &self,
        team: &str,
        id: &str,
        lease: &ScoringLease,
        now: DateTime<Utc>,
    ) -> Result<ScoringLease, EvalError> {
        self.update_lease(team, id, LeaseUpdate::Renew(lease, now))
            .await?
            .ok_or(EvalError::LeaseLost)
    }

    pub async fn release_scoring(
        &self,
        team: &str,
        id: &str,
        lease: &ScoringLease,
    ) -> Result<(), EvalError> {
        self.update_lease(team, id, LeaseUpdate::Release(lease))
            .await?;
        Ok(())
    }

    async fn update_lease(
        &self,
        team: &str,
        id: &str,
        update: LeaseUpdate<'_>,
    ) -> Result<Option<ScoringLease>, EvalError> {
        for attempt in 0..ATTEMPTS {
            let previous = self.state.read(&run_key(team, id)).await?;
            let mut run = stored_run(&previous, team)?;
            if run.run.status != RunStatus::Scoring {
                return Ok(None);
            }
            run.scoring_lease = match update {
                LeaseUpdate::Claim(now) => {
                    if run
                        .scoring_lease
                        .as_ref()
                        .is_some_and(|lease| lease.expires_at > now)
                    {
                        return Ok(None);
                    }
                    Some(ScoringLease {
                        token: uuid::Uuid::new_v4().to_string(),
                        expires_at: now + chrono::Duration::seconds(SCORING_LEASE_SECONDS),
                    })
                }
                LeaseUpdate::Renew(lease, now) => {
                    validate_lease(&run, lease, now)?;
                    Some(ScoringLease {
                        token: lease.token.clone(),
                        expires_at: now + chrono::Duration::seconds(SCORING_LEASE_SECONDS),
                    })
                }
                LeaseUpdate::Release(lease) => {
                    if run
                        .scoring_lease
                        .as_ref()
                        .is_none_or(|current| current.token != lease.token)
                    {
                        return Err(EvalError::LeaseLost);
                    }
                    None
                }
            };
            let value = encode(&run)?;
            if previous.value == value {
                return Ok(run.scoring_lease);
            }
            match self.state.commit(vec![Change { previous, value }]).await {
                Ok(()) => return Ok(run.scoring_lease),
                Err(Error::StateConflict) => retry(attempt).await,
                Err(error) => return Err(error.into()),
            }
        }
        Err(Error::StateConflict.into())
    }

    pub async fn complete(
        &self,
        team: &str,
        id: &str,
        lease: &ScoringLease,
        completion: RunCompletion,
        now: DateTime<Utc>,
    ) -> Result<StoredRun, EvalError> {
        self.close(team, id, lease, Completion::Done(Box::new(completion)), now)
            .await
    }

    pub async fn fail(
        &self,
        team: &str,
        id: &str,
        lease: &ScoringLease,
        failure: &str,
        now: DateTime<Utc>,
    ) -> Result<StoredRun, EvalError> {
        self.close(team, id, lease, Completion::Failed(failure.to_owned()), now)
            .await
    }

    async fn close(
        &self,
        team: &str,
        id: &str,
        lease: &ScoringLease,
        outcome: Completion,
        now: DateTime<Utc>,
    ) -> Result<StoredRun, EvalError> {
        for attempt in 0..ATTEMPTS {
            let previous = self.state.read(&run_key(team, id)).await?;
            let mut run = stored_run(&previous, team)?;
            if matches!(run.run.status, RunStatus::Done | RunStatus::Failed) {
                return Ok(run);
            }
            if run.run.status != RunStatus::Scoring {
                return Err(EvalError::RunClosed);
            }
            validate_lease(&run, lease, now)?;
            match &outcome {
                Completion::Done(completion) => {
                    validate_completion(&run, completion)?;
                    run.run.status = RunStatus::Done;
                    run.run.summary = Some(completion.summary.clone());
                    run.trials = completion.trials.clone();
                    run.verdicts = completion.verdicts.clone();
                }
                Completion::Failed(failure) => {
                    run.run.status = RunStatus::Failed;
                    run.run.failure = failure.clone();
                }
            }
            run.finished_at = Some(now);
            run.scoring_lease = None;
            let index = self.state.read(&scoring_key(team, id)).await?;
            let mut changes = vec![
                Change {
                    previous,
                    value: encode(&run)?,
                },
                Change {
                    previous: index,
                    value: serde_json::Value::Null,
                },
            ];
            if run.run.status == RunStatus::Done && run.request.branch == "main" {
                let key = format!("{}{}_{id}", baseline_prefix(&run)?, descending_time(now));
                changes.push(Change {
                    previous: Snapshot::empty(key),
                    value: encode(&RunReference {
                        team: team.to_owned(),
                        id: id.to_owned(),
                    })?,
                });
            }
            match self.state.commit(changes).await {
                Ok(()) => return Ok(run),
                Err(Error::StateConflict) => retry(attempt).await,
                Err(error) => return Err(error.into()),
            }
        }
        Err(Error::StateConflict.into())
    }
}

enum Completion {
    Done(Box<RunCompletion>),
    Failed(String),
}

enum LeaseUpdate<'a> {
    Claim(DateTime<Utc>),
    Renew(&'a ScoringLease, DateTime<Utc>),
    Release(&'a ScoringLease),
}

fn validate_lease(
    run: &StoredRun,
    lease: &ScoringLease,
    now: DateTime<Utc>,
) -> Result<(), EvalError> {
    if !run
        .scoring_lease
        .as_ref()
        .is_some_and(|current| current.token == lease.token && current.expires_at > now)
    {
        return Err(EvalError::LeaseLost);
    }
    Ok(())
}

fn validate_completion(run: &StoredRun, completion: &RunCompletion) -> Result<(), EvalError> {
    let cases: std::collections::BTreeSet<_> = run.cases.iter().map(|case| &case.id).collect();
    let verdicts: std::collections::BTreeSet<_> = completion.verdicts.keys().collect();
    let trials: std::collections::BTreeSet<_> = completion
        .trials
        .iter()
        .map(|trial| (&trial.case_id, trial.trial))
        .collect();
    let expected_trials: std::collections::BTreeSet<_> = run
        .trials
        .iter()
        .map(|trial| (&trial.case_id, trial.trial))
        .collect();
    if completion.summary.total != run.cases.len() as u64
        || cases != verdicts
        || trials != expected_trials
        || trials.len() != completion.trials.len()
    {
        return Err(EvalError::InvalidRequest(
            "scoring output does not match the eval run",
        ));
    }
    Ok(())
}
