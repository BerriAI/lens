use std::collections::BTreeMap;

use futures_util::{StreamExt, stream};
use lens_contract::eval::{Gate, Scorer, Summary, scorer_names};

use crate::{
    Error, Result,
    baseline::{Baseline, CaseVerdict, Diffs, compare},
    gate::{GateFacts, check_gate},
    scorer::Judge,
    verdict::{Trial, majority, score_trial},
};

const JUDGE_CONCURRENCY: usize = 8;

#[derive(Clone, Debug, PartialEq)]
pub struct CaseInput {
    pub case_id: String,
    pub title: String,
    pub critical: bool,
    pub trials: Vec<Trial>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunInput {
    pub revision: u64,
    pub url: String,
    pub trials: usize,
    pub scorers: Vec<Scorer>,
    pub gate: Gate,
    pub cases: Vec<CaseInput>,
    pub baseline: Option<Baseline>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Evaluation {
    pub summary: Summary,
    pub verdicts: BTreeMap<String, bool>,
}

struct CaseScore {
    passed: bool,
    errors: usize,
    scorer_passes: Vec<usize>,
    cost: f64,
}

fn score_case(run: &RunInput, case: &CaseInput, trials: &[Option<Vec<bool>>]) -> CaseScore {
    let scored: Vec<&Vec<bool>> = trials.iter().flatten().collect();
    CaseScore {
        passed: majority(
            scored
                .iter()
                .filter(|passes| passes.iter().all(|passed| *passed))
                .count(),
            run.trials,
        ),
        errors: trials.len() - scored.len(),
        scorer_passes: (0..run.scorers.len())
            .map(|index| scored.iter().filter(|passes| passes[index]).count())
            .collect(),
        cost: case.trials.iter().map(Trial::cost).sum(),
    }
}

fn validate(run: &RunInput) -> Result<()> {
    if run.scorers.is_empty() || run.cases.is_empty() || run.trials == 0 {
        return Err(Error::EmptyRun);
    }
    let mut ids: Vec<&str> = run.cases.iter().map(|case| case.case_id.as_str()).collect();
    ids.sort_unstable();
    if let Some(pair) = ids.windows(2).find(|pair| pair[0] == pair[1]) {
        return Err(Error::DuplicateCase {
            case_id: pair[0].to_owned(),
        });
    }
    if let Some(case) = run.cases.iter().find(|case| case.trials.len() > run.trials) {
        return Err(Error::ExtraTrials {
            case_id: case.case_id.clone(),
            received: case.trials.len(),
            expected: run.trials,
        });
    }
    match run
        .cases
        .iter()
        .find(|case| !case.trials.iter().all(Trial::has_valid_cost))
    {
        Some(case) => Err(Error::InvalidCost {
            case_id: case.case_id.clone(),
        }),
        None => Ok(()),
    }
}

pub async fn evaluate<J: Judge>(run: &RunInput, judge: &J) -> Result<Evaluation> {
    validate(run)?;
    let slots = run
        .cases
        .iter()
        .flat_map(|case| (0..run.trials).map(move |index| (case, index)));
    let trials: Vec<Option<Vec<bool>>> = stream::iter(slots)
        .map(|(case, index)| {
            score_trial(
                &run.scorers,
                &case.case_id,
                index,
                case.trials.get(index),
                judge,
            )
        })
        .buffered(JUDGE_CONCURRENCY)
        .collect()
        .await;
    let scored: Vec<CaseScore> = run
        .cases
        .iter()
        .zip(trials.chunks(run.trials))
        .map(|(case, trials)| score_case(run, case, trials))
        .collect();
    let total = run.cases.len();
    let passed = scored.iter().filter(|case| case.passed).count();
    let pass_rate = passed as f64 / total as f64;
    let cost_per_case = scored.iter().map(|case| case.cost).sum::<f64>() / total as f64;
    let scores: BTreeMap<String, f64> = scorer_names(&run.scorers)
        .into_iter()
        .enumerate()
        .map(|(index, key)| {
            let passes: usize = scored.iter().map(|case| case.scorer_passes[index]).sum();
            (key, passes as f64 / (total * run.trials) as f64)
        })
        .collect();
    let verdicts: Vec<CaseVerdict<'_>> = run
        .cases
        .iter()
        .zip(&scored)
        .map(|(case, score)| CaseVerdict {
            case_id: &case.case_id,
            title: &case.title,
            critical: case.critical,
            passed: score.passed,
        })
        .collect();
    let Diffs { regressions, fixed } = compare(run.baseline.as_ref(), &run.url, &verdicts);
    let gate = check_gate(
        &run.gate,
        &GateFacts {
            has_baseline: run.baseline.is_some(),
            revision: run.revision,
            regressions: regressions.len(),
            critical_regressions: regressions.iter().filter(|diff| diff.critical).count(),
            pass_rate,
            cost_per_case,
            scores: &scores,
        },
    );
    Ok(Evaluation {
        verdicts: verdicts
            .iter()
            .map(|case| (case.case_id.to_owned(), case.passed))
            .collect(),
        summary: Summary {
            passed: passed as u64,
            total: total as u64,
            pass_rate,
            cost_per_case,
            scores,
            errors: scored.iter().map(|case| case.errors as u64).sum(),
            baseline_run_id: run
                .baseline
                .as_ref()
                .map(|baseline| baseline.run_id.clone()),
            baseline_version: run
                .baseline
                .as_ref()
                .map(|baseline| baseline.version.clone()),
            regressions,
            fixed,
            gate,
        },
    })
}
