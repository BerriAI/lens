use std::collections::BTreeMap;

use lens_contract::eval::{Gate, GateResult};

#[derive(Clone, Copy, Debug)]
pub(crate) struct GateFacts<'a> {
    pub has_baseline: bool,
    pub revision: u64,
    pub regressions: usize,
    pub critical_regressions: usize,
    pub pass_rate: f64,
    pub cost_per_case: f64,
    pub scores: &'a BTreeMap<String, f64>,
}

pub(crate) fn check_gate(gate: &Gate, facts: &GateFacts<'_>) -> GateResult {
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

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn facts(scores: &BTreeMap<String, f64>) -> GateFacts<'_> {
        GateFacts {
            has_baseline: true,
            revision: 4,
            regressions: 3,
            critical_regressions: 1,
            pass_rate: 0.8,
            cost_per_case: 0.2,
            scores,
        }
    }

    fn gate(build: fn(&mut Gate)) -> Gate {
        let mut gate = Gate {
            regressions: None,
            critical: None,
            ..Gate::default()
        };
        build(&mut gate);
        gate
    }

    #[rstest]
    #[case::regressions_over(gate(|g| g.regressions = Some(0)), vec!["3 regressions (max 0)"])]
    #[case::regressions_at_max(gate(|g| g.regressions = Some(3)), vec![])]
    #[case::critical_over(gate(|g| g.critical = Some(0)), vec!["1 critical regressions (max 0)"])]
    #[case::critical_at_max(gate(|g| g.critical = Some(1)), vec![])]
    #[case::pass_rate_below(gate(|g| g.pass_rate = Some(0.9)), vec!["Pass rate below minimum"])]
    #[case::pass_rate_equal(gate(|g| g.pass_rate = Some(0.8)), vec![])]
    #[case::cost_above(gate(|g| g.cost_per_case = Some(0.1)), vec!["Cost per case above maximum"])]
    #[case::cost_equal(gate(|g| g.cost_per_case = Some(0.2)), vec![])]
    #[case::min_below(
        gate(|g| { g.min.insert("judge".into(), 0.95); }),
        vec!["judge below minimum 0.95"]
    )]
    #[case::min_equal(gate(|g| { g.min.insert("judge".into(), 0.9); }), vec![])]
    #[case::min_missing_score(
        gate(|g| { g.min.insert("other".into(), 1.0); }),
        vec!["other below minimum 1"]
    )]
    #[case::all_in_order(
        gate(|g| {
            g.min.insert("z".into(), 1.0);
            g.min.insert("judge".into(), 1.0);
            g.cost_per_case = Some(0.0);
            g.pass_rate = Some(1.0);
            g.critical = Some(0);
            g.regressions = Some(0);
        }),
        vec![
            "3 regressions (max 0)",
            "1 critical regressions (max 0)",
            "Pass rate below minimum",
            "Cost per case above maximum",
            "judge below minimum 1",
            "z below minimum 1",
        ]
    )]
    fn conditions(#[case] gate: Gate, #[case] reasons: Vec<&str>) {
        let scores = BTreeMap::from([("judge".to_owned(), 0.9)]);
        let result = check_gate(&gate, &facts(&scores));
        assert_eq!(result.passed, reasons.is_empty());
        assert_eq!(result.reasons, reasons);
    }

    #[rstest]
    #[case::regressions_ignored(gate(|g| g.regressions = Some(0)), true, vec!["no baseline on main for rev 4"])]
    #[case::critical_ignored(gate(|g| g.critical = Some(0)), true, vec!["no baseline on main for rev 4"])]
    #[case::pass_rate_still_applies(
        gate(|g| g.pass_rate = Some(0.9)),
        false,
        vec!["Pass rate below minimum", "no baseline on main for rev 4"]
    )]
    fn without_baseline(#[case] gate: Gate, #[case] passed: bool, #[case] reasons: Vec<&str>) {
        let scores = BTreeMap::new();
        let result = check_gate(
            &gate,
            &GateFacts {
                has_baseline: false,
                ..facts(&scores)
            },
        );
        assert_eq!(result.passed, passed);
        assert_eq!(result.reasons, reasons);
    }
}
