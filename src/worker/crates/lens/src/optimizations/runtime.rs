use chrono::Utc;
use lens_contract::{feedback::TraceIdentity, investigations::Scope, optimizations::*};
use lens_server::{feedback::FeedbackStore, tracing::TraceReader};
use serde_json::json;

use super::{
    Optimizations, bucket,
    evidence::scope,
    gateway::JudgeEvidence,
    store::{Attempt, AttemptState, Record},
};
use crate::{error::OptimizationError as Error, storage::TraceApi};

impl Optimizations {
    pub fn start(&self) -> tokio::task::JoinHandle<()> {
        let engine = self.clone();
        tokio::spawn(async move {
            let mut cursor = 0;
            loop {
                if engine.tick(cursor).await.is_err() {
                    tracing::warn!("Optimization worker could not complete its current step");
                }
                cursor += 1;
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        })
    }

    pub async fn tick(&self, cursor: usize) -> Result<(), Error> {
        let lease = self.0.store.0.read("optimization-worker").await?;
        let now = Utc::now().timestamp_millis();
        if lease
            .value
            .get("until")
            .and_then(serde_json::Value::as_i64)
            .is_some_and(|until| until > now)
        {
            return Ok(());
        }
        let owner = uuid::Uuid::new_v4().to_string();
        match self
            .0
            .store
            .save(lease, &json!({"owner":owner,"until":now + 600_000}))
            .await
        {
            Err(Error::Storage(litellm_storage_clickhouse::Error::StateConflict)) => return Ok(()),
            result => result?,
        }
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(540), self.work(cursor))
            .await
            .map_err(|_| Error::Gateway)
            .and_then(|result| result);
        let lease = self.0.store.0.read("optimization-worker").await?;
        if lease.value.get("owner").and_then(serde_json::Value::as_str) == Some(&owner) {
            self.0
                .store
                .save(lease, &json!({"owner":owner,"until":0}))
                .await?;
        }
        outcome
    }

    async fn work(&self, cursor: usize) -> Result<(), Error> {
        let records: Vec<_> = self
            .0
            .store
            .records()
            .await?
            .into_iter()
            .filter(|record| record.status != OptimizationStatus::Completed)
            .collect();
        if records.is_empty() {
            return Ok(());
        }
        for record in records.iter().cycle().skip(cursor % records.len()).take(1) {
            if record.status == OptimizationStatus::Completed {
                continue;
            }
            let attempts = self.0.store.attempts(&record.id).await?;
            for (snapshot, mut attempt) in attempts.iter().cloned() {
                if attempt.state == AttemptState::Running
                    && attempt.deadline < Utc::now().timestamp_millis()
                {
                    attempt.result.error = Some("Execution was interrupted. Its provider outcome is unknown; start a new comparison to retry".into());
                    attempt.state = AttemptState::Complete;
                    attempt.result.completed_at = Utc::now().timestamp_millis();
                    self.0.store.save(snapshot, &attempt).await?;
                    return Ok(());
                }
                if record.status != OptimizationStatus::Running {
                    continue;
                }
                if !attempt.external
                    && let Some(mut attempt) = self.0.store.claim(snapshot, attempt).await?
                {
                    if let Err(error) = self.execute(record, &mut attempt).await {
                        attempt.result.error = Some(error.to_string());
                    }
                    self.0.store.finish(&record.id, attempt).await?;
                    return Ok(());
                }
            }
            if record.config.judge == OptimizationJudge::Feedback
                && self.feedback(record, &attempts).await?
            {
                return Ok(());
            }
            if record.status != OptimizationStatus::Running {
                continue;
            }
            if record.config.mode == OptimizationMode::Offline {
                if !attempts.is_empty()
                    && attempts
                        .iter()
                        .all(|(_, attempt)| attempt.state == AttemptState::Complete)
                {
                    self.status(&record.id, OptimizationStatus::Completed)
                        .await?;
                }
                continue;
            }
            if attempts.len() >= 4500 {
                self.status(&record.id, OptimizationStatus::Paused).await?;
                let (snapshot, mut latest) = self.0.store.get(&record.id).await?;
                latest.error = Some("The comparison reached its stored sample limit. Finish it and create another experiment to continue".into());
                self.0.store.save(snapshot, &latest).await?;
                continue;
            }
            if let Err(error) = self.sample(record).await {
                let (snapshot, mut latest) = self.0.store.get(&record.id).await?;
                latest.error = Some(error.to_string());
                latest.status = OptimizationStatus::Paused;
                self.0.store.save(snapshot, &latest).await?;
            }
        }
        Ok(())
    }

    async fn execute(&self, record: &Record, attempt: &mut Attempt) -> Result<(), Error> {
        let gateway = self.gateway()?;
        if let Some(request) = &attempt.request {
            let reply = gateway
                .complete(
                    &attempt.result.model,
                    request,
                    &record.id,
                    &attempt.result.id,
                )
                .await?;
            attempt.result.output = Some(reply.output);
            attempt.result.served_model = Some(reply.model);
            attempt.result.cost_usd = reply.cost;
            attempt.result.duration_ms = Some(reply.duration_ms);
            if let Some(error) = reply.error {
                attempt.result.error = Some(error.into());
                return Ok(());
            }
        }
        if record.config.judge == OptimizationJudge::Feedback {
            return Ok(());
        }
        let sampling = if attempt.result.model == record.config.baseline {
            record.config.baseline_sampling
        } else {
            record.config.candidate_sampling
        };
        if record.config.mode == OptimizationMode::Live
            && bucket(&[&record.id, &attempt.result.source_id, "judge"]) % 100
                >= u64::from(sampling)
        {
            return Ok(());
        }
        let output = attempt
            .result
            .output
            .as_deref()
            .ok_or(Error::Invalid("The run has no final output to judge"))?;
        match gateway
            .judge(JudgeEvidence {
                model: &record.config.judge_model,
                rubric: &record.config.rubric,
                input: &attempt.result.input,
                expected: &attempt.result.expected,
                context: &attempt.context,
                output,
                run: &record.id,
                case: &attempt.result.id,
            })
            .await
        {
            Ok(score) => attempt.result.score = Some(score * 10.0),
            Err(error) => attempt.result.score_error = Some(error.to_string()),
        }
        Ok(())
    }

    async fn feedback(
        &self,
        record: &Record,
        attempts: &[(litellm_storage_clickhouse::state::Snapshot, Attempt)],
    ) -> Result<bool, Error> {
        let complete: Vec<_> = attempts
            .iter()
            .filter(|(_, attempt)| attempt.state == AttemptState::Complete)
            .collect();
        let identities: Vec<_> = complete
            .iter()
            .map(|(_, attempt)| TraceIdentity {
                trace_id: attempt.result.trace_id.clone(),
                trace_ref: attempt.trace_ref.clone(),
            })
            .collect();
        if identities.is_empty() {
            return Ok(false);
        }
        let summaries = crate::FeedbackApi(self.0.state.clone())
            .summaries(
                &Scope {
                    all_teams: false,
                    team_id: record.config.team_id.clone(),
                    ..Default::default()
                },
                &identities,
            )
            .await
            .map_err(|_| Error::Evidence)?;
        for (snapshot, attempt) in complete {
            let score = summaries
                .iter()
                .find(|summary| {
                    summary.trace_id == attempt.result.trace_id
                        && summary.trace_ref == attempt.trace_ref
                })
                .and_then(|summary| summary.average);
            if score != attempt.result.score {
                let mut next = attempt.clone();
                next.result.score = score;
                self.0.store.save(snapshot.clone(), &next).await?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn sample(&self, record: &Record) -> Result<(), Error> {
        let end = if record.scan_cursor.is_none() {
            Utc::now().timestamp_millis()
        } else {
            record.scan_until
        };
        let page = TraceApi(self.0.state.clone())
            .list(
                scope(&record.config.team_id),
                record.config.agent.clone(),
                record.created_at,
                end,
                record.scan_cursor.clone(),
                50,
            )
            .await
            .map_err(|_| Error::Evidence)?;
        for trace in &page.data {
            let assignment = if record.config.mode == OptimizationMode::Live {
                let key = format!(
                    "optimization-assignment/{}/{}",
                    record.id,
                    super::store::digest(&[&trace.trace_id])
                );
                let snapshot = self.0.store.0.read(&key).await?;
                if snapshot.value.is_null() {
                    continue;
                }
                Some(serde_json::from_value::<OptimizationAssignment>(
                    snapshot.value,
                )?)
            } else {
                if bucket(&[&record.id, &trace.trace_id, "traffic"]) % 100
                    >= u64::from(record.config.traffic)
                {
                    continue;
                }
                None
            };
            let Some(source) = self
                .trace_source(record, &trace.trace_id, &trace.trace_ref)
                .await?
            else {
                continue;
            };
            let attempts = match assignment {
                Some(assignment) => {
                    let mut attempt = source.attempt(record, &assignment.model);
                    if source.served_model.as_deref() != Some(&assignment.model) {
                        attempt.result.error =
                            Some("The trace does not confirm the assigned model".into());
                        attempt.state = AttemptState::Complete;
                    }
                    vec![attempt]
                }
                None => {
                    if source.served_model.as_deref() != Some(&record.config.baseline) {
                        continue;
                    }
                    source.attempts(record)
                }
            };
            for attempt in attempts {
                self.0.store.insert_attempt(&record.id, &attempt).await?;
            }
        }
        let (snapshot, mut latest) = self.0.store.get(&record.id).await?;
        latest.scan_cursor = page.next_cursor;
        latest.scan_until = end;
        self.0.store.save(snapshot, &latest).await
    }
}
