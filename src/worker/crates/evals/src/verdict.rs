use lens_contract::eval::Scorer;

use crate::scorer::{self, EvalSpan, Judge};

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
    pub(crate) fn cost(&self) -> f64 {
        match (self.cost_usd, &self.outcome) {
            (Some(cost), _) => cost,
            (None, TrialOutcome::Trace(_)) => self.trace_spend_usd.unwrap_or(0.0),
            (None, TrialOutcome::Error) => 0.0,
        }
    }

    pub(crate) fn has_valid_cost(&self) -> bool {
        [self.cost_usd, self.trace_spend_usd]
            .into_iter()
            .flatten()
            .all(|cost| cost.is_finite() && cost >= 0.0)
    }
}

pub(crate) fn majority(passing: usize, trials: usize) -> bool {
    passing * 2 > trials
}

pub(crate) async fn score_trial<J: Judge>(
    scorers: &[Scorer],
    case_id: &str,
    index: usize,
    trial: Option<&Trial>,
    judge: &J,
) -> Option<Vec<bool>> {
    let Some(TrialOutcome::Trace(spans)) = trial.map(|trial| &trial.outcome) else {
        return None;
    };
    if spans.is_empty() {
        return None;
    }
    let mut passes = Vec::with_capacity(scorers.len());
    for scorer in scorers {
        passes.push(scorer::passes(scorer, case_id, index, spans, judge).await?);
    }
    Some(passes)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::majority;

    #[rstest]
    #[case::one_of_one(1, 1, true)]
    #[case::zero_of_one(0, 1, false)]
    #[case::tie_one_of_two(1, 2, false)]
    #[case::two_of_two(2, 2, true)]
    #[case::two_of_three(2, 3, true)]
    #[case::one_of_three(1, 3, false)]
    #[case::tie_two_of_four(2, 4, false)]
    fn ties_fail(#[case] passing: usize, #[case] trials: usize, #[case] expected: bool) {
        assert_eq!(majority(passing, trials), expected);
    }
}
