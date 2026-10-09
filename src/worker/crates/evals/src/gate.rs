use std::collections::BTreeMap;

use lens_contract::eval::{Gate, GateResult};

#[derive(Clone, Copy, Debug)]
pub struct GateFacts<'a> {
    pub has_baseline: bool,
    pub revision: u64,
    pub regressions: usize,
    pub critical_regressions: usize,
    pub pass_rate: f64,
    pub cost_per_case: f64,
    pub scores: &'a BTreeMap<String, f64>,
}

pub fn check_gate(gate: &Gate, facts: &GateFacts<'_>) -> GateResult {
    let above = |count: usize, max: Option<u64>| {
        facts.has_baseline && max.is_some_and(|max| count as u64 > max)
    };
    let failures: Vec<String> = [
        above(facts.regressions, gate.regressions).then(|| {
            format!(
                "{} regressions (max {})",
                facts.regressions,
                gate.regressions.unwrap_or(0)
            )
        }),
        above(facts.critical_regressions, gate.critical).then(|| {
            format!(
                "{} critical regressions (max {})",
                facts.critical_regressions,
                gate.critical.unwrap_or(0)
            )
        }),
        gate.pass_rate
            .is_some_and(|min| facts.pass_rate < min)
            .then(|| "Pass rate below minimum".to_owned()),
        gate.cost_per_case
            .is_some_and(|max| facts.cost_per_case > max)
            .then(|| "Cost per case above maximum".to_owned()),
    ]
    .into_iter()
    .flatten()
    .chain(
        gate.min
            .iter()
            .filter(|(name, min)| facts.scores.get(*name).is_none_or(|score| score < *min))
            .map(|(name, min)| format!("{name} below minimum {min}")),
    )
    .collect();
    GateResult {
        passed: failures.is_empty(),
        reasons: failures
            .into_iter()
            .chain(
                (!facts.has_baseline)
                    .then(|| format!("no baseline on main for rev {}", facts.revision)),
            )
            .collect(),
    }
}
