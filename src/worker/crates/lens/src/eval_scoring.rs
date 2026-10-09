use lens_evals::{Baseline, CaseInput, RunInput, Trial, TrialOutcome};
use lens_server::eval_closer::{EvalCloserError, RunScoreInput, ScoreRun, ScoredRun};
use litellm_traces_clickhouse::evals::EvalSpan;

use crate::eval_judge::GatewayJudge;

pub struct EvalScorer {
    judge: GatewayJudge,
}

impl EvalScorer {
    pub fn new(judge: GatewayJudge) -> Self {
        Self { judge }
    }
}

impl ScoreRun for EvalScorer {
    async fn score(&self, input: &RunScoreInput) -> Result<ScoredRun, EvalCloserError> {
        let run = RunInput {
            revision: input.run.request.revision,
            url: input.run.run.url.clone(),
            trials: input.run.request.trials as usize,
            scorers: input.run.request.scorers.clone(),
            gate: input.run.request.gate.clone(),
            cases: input
                .run
                .cases
                .iter()
                .map(|case| {
                    let mut trials: Vec<_> = input
                        .trials
                        .iter()
                        .filter(|trial| trial.stored.case_id == case.id)
                        .collect();
                    trials.sort_by_key(|trial| trial.stored.trial);
                    CaseInput {
                        case_id: case.id.clone(),
                        critical: case.critical,
                        trials: trials
                            .into_iter()
                            .map(|trial| Trial {
                                outcome: if trial.stored.result.error.is_some() {
                                    TrialOutcome::Error
                                } else if trial.stored.result.trace.is_some() {
                                    let spans = trial.spans.iter().map(scoring_span).collect();
                                    match &trial.stored.result.output {
                                        Some(output) => TrialOutcome::TraceWithOutput {
                                            spans,
                                            output: output.clone(),
                                        },
                                        None => TrialOutcome::Trace(spans),
                                    }
                                } else if let Some(output) = &trial.stored.result.output {
                                    TrialOutcome::Output(output.clone())
                                } else {
                                    TrialOutcome::Error
                                },
                                cost_usd: trial.stored.result.cost_usd,
                                trace_spend_usd: None,
                            })
                            .collect(),
                    }
                })
                .collect(),
            baseline: input.baseline.as_ref().map(|baseline| Baseline {
                run_id: baseline.run.id.clone(),
                version: baseline.request.version.clone(),
                url: baseline.run.url.clone(),
                verdicts: baseline.verdicts.clone(),
            }),
        };
        let evaluation = lens_evals::evaluate(&run, &self.judge.for_run(input))
            .await
            .map_err(|error| EvalCloserError::Scoring(Box::new(error)))?;
        Ok(ScoredRun {
            summary: evaluation.summary,
            verdicts: evaluation.verdicts,
        })
    }
}

fn scoring_span(span: &EvalSpan) -> lens_evals::EvalSpan {
    lens_evals::EvalSpan {
        span_id: span.span_id.clone(),
        parent_span_id: span.parent_span_id.clone(),
        name: span.name.clone(),
        start_ns: span.start_ns,
        status: match span.status {
            litellm_traces::SpanStatus::Ok => lens_evals::SpanStatus::Ok,
            litellm_traces::SpanStatus::Error => lens_evals::SpanStatus::Error,
            litellm_traces::SpanStatus::Unset => lens_evals::SpanStatus::Unset,
        },
        tool_name: span.attributes.get("gen_ai.tool.name").cloned(),
        input: span.input.clone(),
        output: span.output.clone(),
    }
}
