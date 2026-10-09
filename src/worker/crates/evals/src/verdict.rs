use crate::{
    Result,
    scorer::{EvalSpan, Judge, Scorer},
};

#[derive(Clone, Debug, PartialEq)]
pub enum TrialOutcome {
    Error,
    Trace(Vec<EvalSpan>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Trial {
    pub outcome: TrialOutcome,
    pub cost_usd: Option<f64>,
    pub trace_spend_usd: f64,
}

impl Trial {
    pub fn cost(&self) -> f64 {
        self.cost_usd.unwrap_or(self.trace_spend_usd)
    }
}

pub fn majority(passing: usize, trials: usize) -> bool {
    passing * 2 > trials
}

pub(crate) async fn score_trial<J: Judge>(
    scorers: &[Scorer],
    case_id: &str,
    trial: Option<&Trial>,
    judge: &J,
) -> Result<Option<Vec<bool>>> {
    let Some(TrialOutcome::Trace(spans)) = trial.map(|trial| &trial.outcome) else {
        return Ok(None);
    };
    let mut passes = Vec::with_capacity(scorers.len());
    for scorer in scorers {
        passes.push(scorer.passes(case_id, spans, judge).await?);
    }
    Ok(Some(passes))
}
