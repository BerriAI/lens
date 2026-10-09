mod definitions;
mod records;
mod scoring;

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use lens_contract::eval::{CaseError, CaseResult, CreateEvalRun, EvalRun, RunStatus};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

const QUERY_VALUE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

use crate::{
    Error, EvalError,
    state::{Change, ClickHouseState, Snapshot},
};
pub use records::{
    RunCompletion, RunFilter, ScoringLease, ScoringPage, StoredCase, StoredRun, StoredTrial,
};
use records::{
    RunReference, baseline_prefix, decode, descending_time, digest, encode, retry, run_key,
    scoring_key, stored_run, validate_cases, validate_limit,
};

const ATTEMPTS: u32 = 40;
const PAGE_SIZE: u32 = 100;
const MAX_CASES: usize = 1_000;
pub const SCORING_LEASE_SECONDS: i64 = 60;

fn compatible_baseline(candidate: &StoredRun, baseline: &StoredRun) -> Result<bool, EvalError> {
    Ok(baseline.team == candidate.team
        && baseline.run.id != candidate.run.id
        && baseline.run.status == RunStatus::Done
        && baseline.request.branch == "main"
        && baseline_prefix(baseline)? == baseline_prefix(candidate)?
        && baseline.request.trials == candidate.request.trials
        && baseline
            .cases
            .iter()
            .map(|case| &case.id)
            .collect::<BTreeSet<_>>()
            == candidate
                .cases
                .iter()
                .map(|case| &case.id)
                .collect::<BTreeSet<_>>())
}

#[derive(Clone)]
pub struct EvalStore {
    state: ClickHouseState,
}

impl EvalStore {
    pub fn new(state: ClickHouseState) -> Self {
        Self { state }
    }

    pub fn state(&self) -> &ClickHouseState {
        &self.state
    }

    pub async fn create(
        &self,
        team: &str,
        request: CreateEvalRun,
        cases: Vec<StoredCase>,
        idempotency_key: Option<&str>,
        public_url: &str,
        now: DateTime<Utc>,
    ) -> Result<StoredRun, EvalError> {
        validate_cases(&request, &cases)?;
        if idempotency_key.is_some_and(|key| key.is_empty() || key.len() > 512) {
            return Err(EvalError::InvalidRequest("invalid idempotency key"));
        }
        let id = format!("{}_{}", descending_time(now), uuid::Uuid::new_v4().simple());
        let run = StoredRun {
            team: team.to_owned(),
            run: EvalRun {
                id: id.clone(),
                status: RunStatus::Running,
                eval: request.eval.clone(),
                agent: request.agent.clone(),
                version: request.version.clone(),
                branch: request.branch.clone(),
                pr: request.pr,
                ci_url: String::new(),
                url: format!(
                    "{}/ui/?tab=evals&eval={}&eval_run={id}",
                    public_url.trim_end_matches('/'),
                    utf8_percent_encode(&request.eval, QUERY_VALUE),
                ),
                expected_trials: cases.len() as u64 * u64::from(request.trials),
                received_trials: 0,
                summary: None,
                failure: String::new(),
            },
            request,
            cases,
            trials: Vec::new(),
            verdicts: BTreeMap::new(),
            created_at: now,
            scoring_at: None,
            finished_at: None,
            scoring_lease: None,
        };
        let key = run_key(team, &id);
        if run.request.baseline_run_id.is_some() {
            self.baseline(&run).await?;
        }
        let idempotency = idempotency_key.map(|key| {
            format!(
                "eval-idempotency/{}/{}",
                digest(team.as_bytes()),
                digest(key.as_bytes())
            )
        });
        for attempt in 0..ATTEMPTS {
            let mapping = match &idempotency {
                Some(key) => Some(self.state.read(key).await?),
                None => None,
            };
            if let Some(snapshot) = &mapping
                && !snapshot.value.is_null()
            {
                let reference: RunReference = decode(snapshot)?;
                let existing = self.get(team, &reference.id).await?;
                return if existing.request == run.request {
                    Ok(existing)
                } else {
                    Err(EvalError::IdempotencyConflict)
                };
            }
            let changes = std::iter::once(Change {
                previous: Snapshot::empty(&key),
                value: encode(&run)?,
            })
            .chain(self.missing_definition(team, &run.request, now).await?)
            .chain(
                mapping
                    .map(|previous| -> Result<Change, EvalError> {
                        Ok(Change {
                            previous,
                            value: encode(&RunReference {
                                team: team.to_owned(),
                                id: id.clone(),
                            })?,
                        })
                    })
                    .transpose()?,
            )
            .collect();
            match self.state.commit(changes).await {
                Ok(()) => return Ok(run),
                Err(Error::StateConflict) => retry(attempt).await,
                Err(error) => return Err(error.into()),
            }
        }
        Err(Error::StateConflict.into())
    }

    pub async fn get(&self, team: &str, id: &str) -> Result<StoredRun, EvalError> {
        let snapshot = self.state.read(&run_key(team, id)).await?;
        stored_run(&snapshot, team)
    }

    pub async fn put_result(
        &self,
        team: &str,
        id: &str,
        case_id: &str,
        trial: u32,
        result: CaseResult,
        now: DateTime<Utc>,
    ) -> Result<(), EvalError> {
        result
            .validate()
            .map_err(|_| EvalError::InvalidRequest("invalid case result"))?;
        for attempt in 0..ATTEMPTS {
            let previous = self.state.read(&run_key(team, id)).await?;
            let mut run = stored_run(&previous, team)?;
            if run.run.status != RunStatus::Running {
                return Err(EvalError::RunClosed);
            }
            if !run.cases.iter().any(|case| case.id == case_id) {
                return Err(EvalError::UnknownCase);
            }
            if trial >= run.request.trials {
                return Err(EvalError::InvalidTrial);
            }
            if run.trials.iter().any(|stored| {
                stored.case_id == case_id && stored.trial == trial && stored.result == result
            }) {
                return Ok(());
            }
            let submitted_at = run
                .trials
                .iter()
                .find(|stored| stored.case_id == case_id && stored.trial == trial)
                .map_or(now, |stored| stored.submitted_at);
            run.trials
                .retain(|stored| stored.case_id != case_id || stored.trial != trial);
            run.trials.push(StoredTrial {
                case_id: case_id.to_owned(),
                trial,
                result: result.clone(),
                submitted_at,
            });
            run.trials.sort_by(|left, right| {
                (&left.case_id, left.trial).cmp(&(&right.case_id, right.trial))
            });
            run.run.received_trials = run.trials.len() as u64;
            match self
                .state
                .commit(vec![Change {
                    previous,
                    value: encode(&run)?,
                }])
                .await
            {
                Ok(()) => return Ok(()),
                Err(Error::StateConflict) => retry(attempt).await,
                Err(error) => return Err(error.into()),
            }
        }
        Err(Error::StateConflict.into())
    }

    pub async fn finish(
        &self,
        team: &str,
        id: &str,
        now: DateTime<Utc>,
    ) -> Result<StoredRun, EvalError> {
        for attempt in 0..ATTEMPTS {
            let previous = self.state.read(&run_key(team, id)).await?;
            let mut run = stored_run(&previous, team)?;
            if run.run.status != RunStatus::Running {
                return Err(EvalError::RunClosed);
            }
            let missing: Vec<_> = run
                .cases
                .iter()
                .flat_map(|case| (0..run.request.trials).map(move |trial| (case, trial)))
                .filter(|(case, trial)| {
                    !run.trials
                        .iter()
                        .any(|stored| stored.case_id == case.id && stored.trial == *trial)
                })
                .map(|(case, trial)| StoredTrial {
                    case_id: case.id.clone(),
                    trial,
                    result: CaseResult {
                        error: Some(CaseError {
                            r#type: "missing_result".into(),
                            message: "No result was submitted before finish".into(),
                        }),
                        ..CaseResult::default()
                    },
                    submitted_at: now,
                })
                .collect();
            run.trials.extend(missing);
            run.run.status = RunStatus::Scoring;
            run.scoring_at = Some(now);
            let index = self.state.read(&scoring_key(team, id)).await?;
            let changes = vec![
                Change {
                    previous,
                    value: encode(&run)?,
                },
                Change {
                    previous: index,
                    value: encode(&RunReference {
                        team: team.to_owned(),
                        id: id.to_owned(),
                    })?,
                },
            ];
            match self.state.commit(changes).await {
                Ok(()) => return Ok(run),
                Err(Error::StateConflict) => retry(attempt).await,
                Err(error) => return Err(error.into()),
            }
        }
        Err(Error::StateConflict.into())
    }

    pub async fn list(&self, team: &str, filter: &RunFilter) -> Result<Vec<StoredRun>, EvalError> {
        validate_limit(filter.limit)?;
        let prefix = format!("eval-run/{}/", digest(team.as_bytes()));
        let mut after = filter
            .after
            .as_ref()
            .map_or_else(String::new, |run| format!("{prefix}{run}"));
        let mut runs = Vec::new();
        loop {
            let keys = self.state.keys(&prefix, &after, PAGE_SIZE).await?;
            let Some(last) = keys.last() else {
                return Ok(runs);
            };
            let snapshots = self
                .state
                .read_many(&keys.iter().map(String::as_str).collect::<Vec<_>>())
                .await?;
            for snapshot in snapshots
                .into_iter()
                .filter(|snapshot| !snapshot.value.is_null())
            {
                let run = stored_run(&snapshot, team)?;
                if filter
                    .eval
                    .as_ref()
                    .is_none_or(|eval| *eval == run.request.eval)
                    && filter
                        .agent
                        .as_ref()
                        .is_none_or(|agent| *agent == run.request.agent)
                    && filter
                        .branch
                        .as_ref()
                        .is_none_or(|branch| *branch == run.request.branch)
                    && filter
                        .dataset
                        .as_ref()
                        .is_none_or(|dataset| *dataset == run.request.dataset_id)
                {
                    runs.push(run);
                    if runs.len() == filter.limit as usize {
                        return Ok(runs);
                    }
                }
            }
            after = last.clone();
        }
    }

    pub async fn baseline(&self, candidate: &StoredRun) -> Result<Option<StoredRun>, EvalError> {
        if let Some(id) = &candidate.request.baseline_run_id {
            if id.trim().is_empty() {
                return Err(EvalError::InvalidRequest(
                    "baseline_run_id must be nonempty",
                ));
            }
            let baseline = match self.get(&candidate.team, id).await {
                Ok(run) => run,
                Err(EvalError::RunNotFound) => {
                    return Err(EvalError::InvalidRequest("explicit baseline was not found"));
                }
                Err(error) => return Err(error),
            };
            return if compatible_baseline(candidate, &baseline)? {
                Ok(Some(baseline))
            } else {
                Err(EvalError::InvalidRequest(
                    "explicit baseline must be a completed compatible main run",
                ))
            };
        }
        let prefix = baseline_prefix(candidate)?;
        let mut after = String::new();
        loop {
            let keys = self.state.keys(&prefix, &after, PAGE_SIZE).await?;
            let Some(last) = keys.last() else {
                return Ok(None);
            };
            let snapshots = self
                .state
                .read_many(&keys.iter().map(String::as_str).collect::<Vec<_>>())
                .await?;
            for snapshot in snapshots
                .into_iter()
                .filter(|snapshot| !snapshot.value.is_null())
            {
                let reference: RunReference = decode(&snapshot)?;
                if reference.id == candidate.run.id {
                    continue;
                }
                let run = self.get(&candidate.team, &reference.id).await?;
                if compatible_baseline(candidate, &run)? {
                    return Ok(Some(run));
                }
            }
            after = last.clone();
        }
    }

    pub async fn scoring(&self, after: &str, limit: u32) -> Result<ScoringPage, EvalError> {
        validate_limit(limit)?;
        let keys = self.state.keys("eval-scoring/", after, limit).await?;
        let next = if keys.len() == limit as usize {
            keys.last().cloned()
        } else {
            None
        };
        let snapshots = self
            .state
            .read_many(&keys.iter().map(String::as_str).collect::<Vec<_>>())
            .await?;
        let mut runs = Vec::new();
        for snapshot in snapshots
            .into_iter()
            .filter(|snapshot| !snapshot.value.is_null())
        {
            let reference: RunReference = decode(&snapshot)?;
            let run = self.get(&reference.team, &reference.id).await?;
            if run.run.status == RunStatus::Scoring {
                runs.push(run);
            }
        }
        Ok(ScoringPage { runs, next })
    }
}
