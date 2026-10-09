use std::{collections::BTreeMap, time::Duration};

use chrono::{DateTime, TimeDelta, Utc};
use futures_util::{StreamExt, stream};
use lens_contract::{
    feedback::TraceIdentity,
    investigations::Scope,
    signals::{SignalAttemptStatus, SignalConfig},
    worker::Execution,
};

use crate::{
    Decisions, Error, SignalReader, SignalRepository, candidate, classify, config_key, enabled,
};

pub const MAX_PER_TICK: usize = 50;

#[derive(Clone, Copy, Debug)]
pub struct SignalSweep {
    pub lookback: TimeDelta,
    pub interval: Duration,
    pub max_pages: usize,
}

pub const LIVE_SWEEP: SignalSweep = SignalSweep {
    lookback: TimeDelta::minutes(15),
    interval: Duration::from_secs(2),
    max_pages: 1,
};
pub const BACKLOG_SWEEP: SignalSweep = SignalSweep {
    lookback: TimeDelta::hours(24),
    interval: Duration::from_secs(60),
    max_pages: 10,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SignalTick {
    pub cursor: String,
    pub claimed: usize,
}

struct Scan<'a, S, R> {
    reader: &'a S,
    repository: &'a R,
    scope: &'a Scope,
    config: &'a SignalConfig,
    now: DateTime<Utc>,
    sweep: SignalSweep,
}

impl<S: SignalReader, R: SignalRepository> Scan<'_, S, R> {
    async fn run(&self, mut cursor: String) -> Result<(Vec<Execution>, String), Error> {
        let start = (self.now - self.sweep.lookback).timestamp_millis();
        let end = (self.now - TimeDelta::seconds(15)).timestamp_millis();
        let key = config_key(self.config);
        let mut executions = Vec::new();
        for _ in 0..self.sweep.max_pages {
            if executions.len() >= MAX_PER_TICK {
                break;
            }
            let page = self
                .reader
                .sample(self.scope, start, end, 100, &cursor)
                .await?;
            let identities = page
                .executions
                .iter()
                .map(|execution| TraceIdentity {
                    trace_id: execution.trace_id.clone(),
                    trace_ref: execution.trace_ref.clone(),
                })
                .collect::<Vec<_>>();
            let existing = self
                .repository
                .traces(&identities)
                .await?
                .into_iter()
                .map(|row| ((row.trace_id.clone(), row.trace_ref.clone()), row))
                .collect::<BTreeMap<_, _>>();
            let eligible = page
                .executions
                .into_iter()
                .filter(|execution| {
                    candidate(
                        execution,
                        existing.get(&(execution.trace_id.clone(), execution.trace_ref.clone())),
                        &key,
                        self.now,
                    )
                })
                .collect::<Vec<_>>();
            let remaining = MAX_PER_TICK - executions.len();
            let capped = eligible.len() > remaining;
            executions.extend(eligible.into_iter().take(remaining));
            if capped {
                break;
            }
            let Some(next) = page.next_cursor else {
                cursor.clear();
                break;
            };
            cursor = next;
        }
        Ok((executions, cursor))
    }
}

pub async fn run_signal_tick<S: SignalReader, R: SignalRepository, D: Decisions>(
    reader: &S,
    repository: Option<&R>,
    completion: Option<&D>,
    clock: &(impl Fn() -> DateTime<Utc> + Sync),
    ready: bool,
    cursor: &str,
    sweep: SignalSweep,
) -> Result<SignalTick, Error> {
    let unchanged = SignalTick {
        cursor: cursor.into(),
        claimed: 0,
    };
    let (Some(repository), Some(completion), true) = (repository, completion, ready) else {
        return Ok(unchanged);
    };
    let now = clock();
    let config = repository.get_config().await?;
    if !enabled(&config) {
        return Ok(unchanged);
    }
    let scope = Scope {
        all_teams: true,
        team_id: String::new(),
        api_key_hash: String::new(),
    };
    let (executions, cursor) = Scan {
        reader,
        repository,
        scope: &scope,
        config: &config,
        now,
        sweep,
    }
    .run(cursor.into())
    .await?;
    let claimed = stream::iter(executions)
        .map(|execution| {
            let config = &config;
            let scope = &scope;
            async move {
                let now = clock();
                let lease = now + TimeDelta::minutes(5);
                match repository.claim(&execution, config, lease, now).await {
                    Ok(true) => (),
                    Ok(false) => return 0,
                    Err(error) => {
                        tracing::error!(%error, "Lens signal claim failed");
                        return 0;
                    }
                }
                let attempt = classify(reader, completion, scope, &execution, config).await;
                if attempt.status == SignalAttemptStatus::Failed {
                    tracing::warn!(reason = %attempt.error, "Lens signal classification failed");
                }
                if let Err(error) = repository
                    .store(&execution, config, lease, clock(), &attempt)
                    .await
                {
                    tracing::error!(%error, "Lens signal result could not be stored");
                }
                1
            }
        })
        .buffer_unordered(8)
        .fold(0, |count, claimed| async move { count + claimed })
        .await;
    Ok(SignalTick { cursor, claimed })
}
