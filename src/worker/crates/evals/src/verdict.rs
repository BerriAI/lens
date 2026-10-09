use lens_contract::eval::Scorer;

use crate::{
    Result,
    scorer::{self, EvalSpan, Judge},
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
    pub trace_spend_usd: Option<f64>,
}

impl Trial {
    pub fn cost(&self) -> f64 {
        match (self.cost_usd, &self.outcome) {
            (Some(cost), _) => cost,
            (None, TrialOutcome::Trace(_)) => self.trace_spend_usd.unwrap_or(0.0),
            (None, TrialOutcome::Error) => 0.0,
        }
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
        passes.push(scorer::passes(scorer, case_id, spans, judge).await?);
    }
    Ok(Some(passes))
}
