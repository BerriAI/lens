use std::{collections::BTreeMap, future::Future, time::Duration};

use chrono::{DateTime, Utc};
use futures_util::{StreamExt, TryStreamExt, stream};
use lens_contract::eval::{CaseError, CaseResult, RunStatus, Summary, TraceRef};
use litellm_storage_clickhouse::evals::{
    EvalStore, RunCompletion, ScoringLease, StoredRun, StoredTrial,
};
use litellm_traces_clickhouse::evals::{EvalSpan, EvalTrace};

pub use crate::error::EvalCloserError;

const TRACE_IDLE_MS: i64 = 120_000;
const TRACE_LOOKUP_CONCURRENCY: usize = 16;

pub trait TraceSource: Send + Sync {
    fn trace(
        &self,
        team: &str,
        reference: &TraceRef,
    ) -> impl Future<Output = Result<Option<EvalTrace>, EvalCloserError>> + Send;
}

pub struct ResolvedTrial {
    pub stored: StoredTrial,
    pub spans: Vec<EvalSpan>,
}

pub struct RunScoreInput {
    pub run: StoredRun,
    pub trials: Vec<ResolvedTrial>,
    pub baseline: Option<StoredRun>,
}

pub struct ScoredRun {
    pub summary: Summary,
    pub verdicts: BTreeMap<String, bool>,
}

pub trait ScoreRun: Send + Sync {
    fn score(
        &self,
        input: &RunScoreInput,
    ) -> impl Future<Output = Result<ScoredRun, EvalCloserError>> + Send;
}

pub struct UnavailableScorer;

impl ScoreRun for UnavailableScorer {
    async fn score(&self, _input: &RunScoreInput) -> Result<ScoredRun, EvalCloserError> {
        Err(EvalCloserError::ScoringUnavailable)
    }
}

pub struct EvalCloser<T, S> {
    store: EvalStore,
    traces: T,
    scorer: S,
}

impl<T: TraceSource, S: ScoreRun> EvalCloser<T, S> {
    pub fn new(store: EvalStore, traces: T, scorer: S) -> Self {
        Self {
            store,
            traces,
            scorer,
        }
    }

    pub async fn tick(&self, now: DateTime<Utc>) -> Result<(), EvalCloserError> {
        let mut after = String::new();
        loop {
            let page = self.store.scoring(&after, 100).await?;
            for run in page.runs {
                self.advance(&run, now).await?;
            }
            match page.next {
                Some(next) => after = next,
                None => return Ok(()),
            }
        }
    }

    pub async fn advance(
        &self,
        run: &StoredRun,
        now: DateTime<Utc>,
    ) -> Result<(), EvalCloserError> {
        if run.run.status != RunStatus::Scoring {
            return Ok(());
        }
        let resolved = resolve_trials(&self.traces, run, now).await;
        let trials = match resolved {
            Ok(Some(trials)) => trials,
            Ok(None) => return Ok(()),
            Err(error) => {
                if let Some(lease) = self
                    .store
                    .claim_scoring(&run.team, &run.run.id, now)
                    .await?
                {
                    self.fail(run, &lease, &error, now).await?;
                }
                return Ok(());
            }
        };
        let baseline = self.store.baseline(run).await?;
        let Some(lease) = self
            .store
            .claim_scoring(&run.team, &run.run.id, now.max(Utc::now()))
            .await?
        else {
            return Ok(());
        };
        let input = RunScoreInput {
            run: run.clone(),
            trials,
            baseline,
        };
        match self.score_with_lease(&input, &lease).await {
            Ok(scored) => {
                self.store
                    .complete(
                        &run.team,
                        &run.run.id,
                        &lease,
                        RunCompletion {
                            summary: scored.summary,
                            trials: input.trials.into_iter().map(|trial| trial.stored).collect(),
                            verdicts: scored.verdicts,
                        },
                        now.max(Utc::now()),
                    )
                    .await?;
                Ok(())
            }
            Err(error) => self.fail(run, &lease, &error, now.max(Utc::now())).await,
        }
    }

    async fn score_with_lease(
        &self,
        input: &RunScoreInput,
        lease: &ScoringLease,
    ) -> Result<ScoredRun, EvalCloserError> {
        let scoring = self.scorer.score(input);
        tokio::pin!(scoring);
        let mut heartbeat = tokio::time::interval(Duration::from_secs(20));
        heartbeat.tick().await;
        loop {
            tokio::select! {
                scored = &mut scoring => return scored,
                _ = heartbeat.tick() => {
                    self.store.renew_scoring(&input.run.team, &input.run.run.id, lease, Utc::now()).await?;
                }
            }
        }
    }

    async fn fail(
        &self,
        run: &StoredRun,
        lease: &ScoringLease,
        error: &EvalCloserError,
        now: DateTime<Utc>,
    ) -> Result<(), EvalCloserError> {
        self.store
            .fail(&run.team, &run.run.id, lease, &error.to_string(), now)
            .await?;
        Ok(())
    }
}

async fn resolve_trials<T: TraceSource>(
    traces: &T,
    run: &StoredRun,
    now: DateTime<Utc>,
) -> Result<Option<Vec<ResolvedTrial>>, EvalCloserError> {
    if run.trials.len() as u64 != run.run.expected_trials {
        return Err(EvalCloserError::InvalidRun);
    }
    let pending = run
        .trials
        .iter()
        .map(|trial| async move {
            let trace = match &trial.result.trace {
                Some(reference) => traces.trace(&run.team, reference).await?,
                None => None,
            };
            Ok::<_, EvalCloserError>(resolve_trial(run, trial, trace, now.timestamp_millis()))
        })
        .collect::<Vec<_>>();
    let resolved: Vec<Option<ResolvedTrial>> = stream::iter(pending)
        .buffered(TRACE_LOOKUP_CONCURRENCY)
        .try_collect()
        .await?;
    Ok(resolved.into_iter().collect())
}

fn resolve_trial(
    run: &StoredRun,
    trial: &StoredTrial,
    trace: Option<EvalTrace>,
    now_ms: i64,
) -> Option<ResolvedTrial> {
    if trial.result.error.is_some() {
        return Some(ResolvedTrial {
            stored: trial.clone(),
            spans: Vec::new(),
        });
    }
    let remaining = run
        .request
        .timeout_per_trial_ms
        .saturating_sub(trial.result.duration_ms.unwrap_or(0));
    let deadline_ms = trial
        .submitted_at
        .timestamp_millis()
        .saturating_add(i64::try_from(remaining).unwrap_or(i64::MAX));
    if let Some(trace) = trace.filter(|trace| !trace.spans.is_empty()) {
        if trace.agent != run.request.agent {
            return Some(trial_error(
                trial,
                "AgentMismatch",
                "Trace agent.name differs from the eval run agent",
            ));
        }
        if trace.version != run.request.version {
            return Some(trial_error(
                trial,
                "VersionMismatch",
                "Trace agent.version differs from the eval run version",
            ));
        }
        if trace.environment != "lens-eval" {
            return Some(trial_error(
                trial,
                "EnvironmentMismatch",
                "Trace deployment.environment must be lens-eval",
            ));
        }
        let idle_at_ms = trace.last_received_at_ms.saturating_add(TRACE_IDLE_MS);
        let closed_at_ms = trace
            .root_ended_at_ms
            .map_or(idle_at_ms, |root| root.min(idle_at_ms));
        let stored = StoredTrial {
            result: CaseResult {
                cost_usd: trial.result.cost_usd.or(Some(trace.gateway_cost_usd)),
                ..trial.result.clone()
            },
            ..trial.clone()
        };
        if closed_at_ms <= now_ms && closed_at_ms <= deadline_ms {
            return Some(ResolvedTrial {
                stored,
                spans: trace.spans,
            });
        }
        return (now_ms >= deadline_ms).then(|| timeout_error(&stored));
    }
    (now_ms >= deadline_ms).then(|| timeout_error(trial))
}

fn timeout_error(trial: &StoredTrial) -> ResolvedTrial {
    trial_error(
        trial,
        "TimeoutError",
        "Trace did not close before the trial timeout",
    )
}

fn trial_error(trial: &StoredTrial, kind: &str, message: &str) -> ResolvedTrial {
    ResolvedTrial {
        stored: StoredTrial {
            result: CaseResult {
                trace: None,
                error: Some(CaseError {
                    r#type: kind.to_owned(),
                    message: message.to_owned(),
                }),
                ..trial.result.clone()
            },
            ..trial.clone()
        },
        spans: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use lens_contract::eval::{
        CreateEvalRun, EvalRun, Gate, Scorer, TaskCompleted, TraceAttribute,
    };
    use litellm_storage_clickhouse::evals::StoredCase;
    use litellm_traces::SpanStatus;
    use rstest::{fixture, rstest};

    use super::*;

    #[fixture]
    fn submitted_at() -> DateTime<Utc> {
        DateTime::from_timestamp_millis(1_000_000).unwrap()
    }

    #[fixture]
    fn trial(submitted_at: DateTime<Utc>) -> StoredTrial {
        StoredTrial {
            case_id: "case-1".into(),
            trial: 0,
            result: CaseResult {
                trace: Some(TraceRef {
                    attribute: TraceAttribute::SessionId,
                    value: "accepted-run".into(),
                }),
                ..CaseResult::default()
            },
            submitted_at,
        }
    }

    #[fixture]
    fn run(trial: StoredTrial, submitted_at: DateTime<Utc>) -> StoredRun {
        let request = CreateEvalRun {
            eval: "regressions".into(),
            agent: "agent".into(),
            dataset_id: "dataset".into(),
            revision: 7,
            case_ids: None,
            version: "sha".into(),
            branch: "feature".into(),
            pr: None,
            ci_url: String::new(),
            trials: 1,
            scorers: vec![Scorer::TaskCompleted(TaskCompleted {})],
            gate: Gate::default(),
            timeout_per_trial_ms: 1_200_000,
        };
        StoredRun {
            team: "team-a".into(),
            run: EvalRun {
                id: "run-1".into(),
                status: RunStatus::Scoring,
                eval: request.eval.clone(),
                agent: request.agent.clone(),
                version: request.version.clone(),
                branch: request.branch.clone(),
                pr: None,
                url: "http://localhost/ui/?run=run-1".into(),
                expected_trials: 1,
                received_trials: 1,
                summary: None,
                failure: String::new(),
            },
            request,
            cases: vec![StoredCase {
                id: trial.case_id.clone(),
                title: "case-1".into(),
                critical: false,
                expected: String::new(),
            }],
            trials: vec![trial],
            verdicts: BTreeMap::new(),
            created_at: submitted_at,
            scoring_at: Some(submitted_at),
            scoring_lease: None,
            finished_at: None,
        }
    }

    #[fixture]
    fn trace(submitted_at: DateTime<Utc>) -> EvalTrace {
        EvalTrace {
            spans: vec![EvalSpan {
                span_id: "root".into(),
                parent_span_id: String::new(),
                name: "agent".into(),
                start_ns: submitted_at.timestamp_nanos_opt().unwrap(),
                end_ns: submitted_at.timestamp_nanos_opt().unwrap(),
                status: SpanStatus::Ok,
                attributes: BTreeMap::new(),
                input: "task".into(),
                output: "finished".into(),
            }],
            root_ended_at_ms: Some(submitted_at.timestamp_millis()),
            last_received_at_ms: submitted_at.timestamp_millis(),
            agent: "agent".into(),
            version: "sha".into(),
            environment: "lens-eval".into(),
            gateway_cost_usd: 0.12,
        }
    }

    #[rstest]
    #[case::root_ended(Some(0), 0, 0, 1_200_000, Some("closed"))]
    #[case::root_not_yet_ended(Some(1), 0, 0, 1_200_000, None)]
    #[case::awaiting_idle(None, 0, 119_999, 1_200_000, None)]
    #[case::idle_closed(None, 0, 120_000, 1_200_000, Some("closed"))]
    #[case::new_span_resets_idle(None, 10_000, 120_000, 1_200_000, None)]
    #[case::timeout_before_idle(None, 0, 60_000, 60_000, Some("TimeoutError"))]
    #[case::root_after_deadline(Some(60_001), 0, 120_000, 60_000, Some("TimeoutError"))]
    #[case::root_at_deadline(Some(60_000), 0, 120_000, 60_000, Some("closed"))]
    fn closure_respects_arrivals_and_timeout(
        mut run: StoredRun,
        mut trace: EvalTrace,
        #[case] ended_after_ms: Option<i64>,
        #[case] arrival_after_ms: i64,
        #[case] elapsed_ms: i64,
        #[case] timeout_ms: u64,
        #[case] expected: Option<&str>,
    ) {
        let trial = &run.trials[0];
        let start = trial.submitted_at.timestamp_millis();
        trace.root_ended_at_ms = ended_after_ms.map(|elapsed| start + elapsed);
        trace.last_received_at_ms = start + arrival_after_ms;
        run.request.timeout_per_trial_ms = timeout_ms;
        let resolved = resolve_trial(&run, trial, Some(trace), start + elapsed_ms);
        assert_eq!(
            resolved.as_ref().map(|trial| trial
                .stored
                .result
                .error
                .as_ref()
                .map_or("closed", |error| error.r#type.as_str())),
            expected
        );
    }

    #[rstest]
    #[case::waiting(0, 999, None)]
    #[case::never_arrived(0, 1000, Some("TimeoutError"))]
    #[case::task_used_budget(900, 100, Some("TimeoutError"))]
    #[case::task_exhausted_budget(1000, 0, Some("TimeoutError"))]
    fn missing_traces_use_remaining_trial_budget(
        mut run: StoredRun,
        mut trial: StoredTrial,
        #[case] duration_ms: u64,
        #[case] elapsed_ms: i64,
        #[case] expected: Option<&str>,
    ) {
        run.request.timeout_per_trial_ms = 1000;
        trial.result.duration_ms = Some(duration_ms);
        let resolved = resolve_trial(
            &run,
            &trial,
            None,
            trial.submitted_at.timestamp_millis() + elapsed_ms,
        );
        assert_eq!(
            resolved.as_ref().map(|trial| trial
                .stored
                .result
                .error
                .as_ref()
                .unwrap()
                .r#type
                .as_str()),
            expected
        );
        if let Some(resolved) = resolved {
            assert!(resolved.stored.result.trace.is_none());
            assert!(resolved.spans.is_empty());
            assert!(resolved.stored.result.validate().is_ok());
        }
    }

    #[rstest]
    #[case::agent("other-agent", "sha", "lens-eval", "AgentMismatch")]
    #[case::version("agent", "stale-sha", "lens-eval", "VersionMismatch")]
    #[case::production("agent", "sha", "production", "EnvironmentMismatch")]
    fn traces_must_match_the_eval_identity(
        run: StoredRun,
        trial: StoredTrial,
        mut trace: EvalTrace,
        #[case] agent: &str,
        #[case] version: &str,
        #[case] environment: &str,
        #[case] expected: &str,
    ) {
        trace.agent = agent.into();
        trace.version = version.into();
        trace.environment = environment.into();
        let result = resolve_trial(
            &run,
            &trial,
            Some(trace),
            trial.submitted_at.timestamp_millis(),
        )
        .unwrap();
        assert_eq!(result.stored.result.error.unwrap().r#type, expected);
        assert!(result.stored.result.trace.is_none());
    }

    #[rstest]
    #[case::explicit(Some(0.04), 0.04)]
    #[case::gateway_fallback(None, 0.12)]
    fn closed_trace_preserves_spans_and_resolves_cost(
        run: StoredRun,
        mut trial: StoredTrial,
        trace: EvalTrace,
        #[case] supplied: Option<f64>,
        #[case] expected: f64,
    ) {
        trial.result.cost_usd = supplied;
        let resolved = resolve_trial(
            &run,
            &trial,
            Some(trace),
            trial.submitted_at.timestamp_millis(),
        )
        .unwrap();
        assert_eq!(resolved.stored.result.cost_usd, Some(expected));
        assert_eq!(resolved.spans.len(), 1);
        assert_eq!(resolved.spans[0].span_id, "root");
        assert!(resolved.stored.result.error.is_none());
    }

    #[rstest]
    fn timed_out_trace_preserves_incurred_gateway_spend(
        mut run: StoredRun,
        trial: StoredTrial,
        mut trace: EvalTrace,
    ) {
        run.request.timeout_per_trial_ms = 1000;
        trace.root_ended_at_ms = None;
        let resolved = resolve_trial(
            &run,
            &trial,
            Some(trace),
            trial.submitted_at.timestamp_millis() + 1000,
        )
        .unwrap();
        assert_eq!(resolved.stored.result.error.unwrap().r#type, "TimeoutError");
        assert_eq!(resolved.stored.result.cost_usd, Some(0.12));
    }

    struct MissingTrace;

    impl TraceSource for MissingTrace {
        async fn trace(
            &self,
            team: &str,
            reference: &TraceRef,
        ) -> Result<Option<EvalTrace>, EvalCloserError> {
            assert_eq!(team, "team-a");
            assert_eq!(reference.attribute, TraceAttribute::SessionId);
            Ok(None)
        }
    }

    #[rstest]
    #[case::ordinary("accepted-run")]
    #[case::synthetic_looking("pass-but-missing")]
    #[tokio::test]
    async fn never_arriving_trace_reaches_scorer_as_error(
        mut run: StoredRun,
        #[case] reference: &str,
    ) {
        run.trials[0].result.trace.as_mut().unwrap().value = reference.into();
        let now = run.created_at
            + chrono::Duration::milliseconds(run.request.timeout_per_trial_ms as i64);
        let resolved = resolve_trials(&MissingTrace, &run, now)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(
            resolved[0].stored.result.error.as_ref().unwrap().r#type,
            "TimeoutError"
        );
        assert!(resolved[0].stored.result.trace.is_none());
    }

    #[rstest]
    #[tokio::test]
    async fn unresolved_trial_defers_entire_run(run: StoredRun) {
        let resolved =
            tokio::spawn(async move { resolve_trials(&MissingTrace, &run, run.created_at).await })
                .await
                .unwrap()
                .unwrap();
        assert!(resolved.is_none());
    }

    #[rstest]
    #[tokio::test]
    async fn incomplete_persisted_trials_are_not_silently_scored(mut run: StoredRun) {
        run.trials.clear();
        assert!(matches!(
            resolve_trials(&MissingTrace, &run, run.created_at).await,
            Err(EvalCloserError::InvalidRun)
        ));
    }

    #[rstest]
    #[tokio::test]
    async fn unavailable_upstream_scorer_cannot_invent_a_pass(run: StoredRun) {
        let input = RunScoreInput {
            run,
            trials: Vec::new(),
            baseline: None,
        };
        assert!(matches!(
            UnavailableScorer.score(&input).await,
            Err(EvalCloserError::ScoringUnavailable)
        ));
    }
}
