use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    Error, Result,
    baseline::{Baseline, CaseDiff, CaseVerdict, Diffs, compare},
    gate::{Gate, GateFacts, GateResult, check_gate},
    scorer::{Judge, Scorer, scorer_keys},
    verdict::{Trial, majority, score_trial},
};

#[derive(Clone, Debug, PartialEq)]
pub struct CaseInput {
    pub case_id: String,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    pub passed: usize,
    pub total: usize,
    pub pass_rate: f64,
    pub cost_per_case: f64,
    pub scores: BTreeMap<String, f64>,
    pub errors: usize,
    pub baseline_run_id: Option<String>,
    pub baseline_version: Option<String>,
    pub regressions: Vec<CaseDiff>,
    pub fixed: Vec<CaseDiff>,
    pub gate: GateResult,
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

async fn score_case<J: Judge>(run: &RunInput, case: &CaseInput, judge: &J) -> Result<CaseScore> {
    let mut passing = 0;
    let mut errors = 0;
    let mut scorer_passes = vec![0; run.scorers.len()];
    for index in 0..run.trials {
        let Some(passes) =
            score_trial(&run.scorers, &case.case_id, case.trials.get(index), judge).await?
        else {
            errors += 1;
            continue;
        };
        passing += usize::from(passes.iter().all(|passed| *passed));
        scorer_passes
            .iter_mut()
            .zip(&passes)
            .for_each(|(count, passed)| *count += usize::from(*passed));
    }
    Ok(CaseScore {
        passed: majority(passing, run.trials),
        errors,
        scorer_passes,
        cost: case.trials.iter().map(Trial::cost).sum(),
    })
}

fn validate(run: &RunInput) -> Result<()> {
    if run.cases.is_empty() || run.trials == 0 {
        return Err(Error::EmptyRun);
    }
    match run.cases.iter().find(|case| case.trials.len() > run.trials) {
        Some(case) => Err(Error::ExtraTrials {
            case_id: case.case_id.clone(),
            received: case.trials.len(),
            expected: run.trials,
        }),
        None => Ok(()),
    }
}

pub async fn evaluate<J: Judge>(run: &RunInput, judge: &J) -> Result<Evaluation> {
    validate(run)?;
    let mut scored = Vec::with_capacity(run.cases.len());
    for case in &run.cases {
        scored.push(score_case(run, case, judge).await?);
    }
    let total = run.cases.len();
    let passed = scored.iter().filter(|case| case.passed).count();
    let pass_rate = passed as f64 / total as f64;
    let cost_per_case = scored.iter().map(|case| case.cost).sum::<f64>() / total as f64;
    let scores: BTreeMap<String, f64> = scorer_keys(&run.scorers)
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
            passed,
            total,
            pass_rate,
            cost_per_case,
            scores,
            errors: scored.iter().map(|case| case.errors).sum(),
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
