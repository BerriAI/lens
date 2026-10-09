#![allow(
    dead_code,
    reason = "Each integration test binary uses a different subset of these helpers"
)]

use std::collections::BTreeMap;

use lens_evals::{
    Baseline, CaseInput, EvalSpan, Gate, Judge, JudgeError, JudgeRequest, RunInput, Scorer,
    SpanStatus, Trial, TrialOutcome,
};

pub struct FakeJudge(pub BTreeMap<String, f64>);

impl FakeJudge {
    pub fn constant(score: f64) -> Self {
        Self(BTreeMap::from([(String::new(), score)]))
    }
}

impl Judge for FakeJudge {
    async fn score(&self, request: JudgeRequest<'_>) -> Result<f64, JudgeError> {
        self.0
            .get(request.prompt)
            .or_else(|| self.0.get(""))
            .copied()
            .ok_or_else(|| format!("no score for prompt {}", request.prompt).into())
    }
}

pub fn span(id: &str, parent: &str, start_ns: i64, status: SpanStatus) -> EvalSpan {
    EvalSpan {
        span_id: id.into(),
        parent_span_id: parent.into(),
        name: id.into(),
        start_ns,
        status,
        tool_name: None,
    }
}

pub fn tool(id: &str, tool_name: &str, start_ns: i64) -> EvalSpan {
    EvalSpan {
        tool_name: Some(tool_name.into()),
        ..span(id, "root", start_ns, SpanStatus::Ok)
    }
}

pub fn root(status: SpanStatus) -> EvalSpan {
    span("root", "", 0, status)
}

pub fn trace(status: SpanStatus, cost_usd: Option<f64>) -> Trial {
    Trial {
        outcome: TrialOutcome::Trace(vec![root(status)]),
        cost_usd,
        trace_spend_usd: 0.0,
    }
}

pub fn pass() -> Trial {
    trace(SpanStatus::Ok, Some(0.01))
}

pub fn fail() -> Trial {
    trace(SpanStatus::Error, Some(0.01))
}

pub fn error() -> Trial {
    Trial {
        outcome: TrialOutcome::Error,
        cost_usd: None,
        trace_spend_usd: 0.0,
    }
}

pub fn case(id: &str, critical: bool, trials: Vec<Trial>) -> CaseInput {
    CaseInput {
        case_id: id.into(),
        critical,
        trials,
    }
}

pub fn baseline(verdicts: &[(&str, bool)]) -> Baseline {
    Baseline {
        run_id: "baseline".into(),
        version: "base".into(),
        url: "http://lens/runs/baseline".into(),
        verdicts: verdicts
            .iter()
            .map(|(id, passed)| ((*id).to_owned(), *passed))
            .collect(),
    }
}

pub fn run(trials: usize, cases: Vec<CaseInput>, baseline: Option<Baseline>) -> RunInput {
    RunInput {
        revision: 1,
        url: "http://lens/runs/candidate".into(),
        trials,
        scorers: vec![Scorer::TaskCompleted],
        gate: Gate::default(),
        cases,
        baseline,
    }
}
