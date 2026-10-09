use std::collections::BTreeMap;

use lens_contract::eval::CaseDiff;

#[derive(Clone, Debug, PartialEq)]
pub struct Baseline {
    pub run_id: String,
    pub version: String,
    pub url: String,
    pub verdicts: BTreeMap<String, bool>,
}

pub(crate) struct CaseVerdict<'a> {
    pub case_id: &'a str,
    pub critical: bool,
    pub passed: bool,
}

pub(crate) struct Diffs {
    pub regressions: Vec<CaseDiff>,
    pub fixed: Vec<CaseDiff>,
}

pub(crate) fn compare(
    baseline: Option<&Baseline>,
    candidate_url: &str,
    cases: &[CaseVerdict<'_>],
) -> Diffs {
    let Some(baseline) = baseline else {
        return Diffs {
            regressions: Vec::new(),
            fixed: Vec::new(),
        };
    };
    let changed = |was: bool| {
        cases
            .iter()
            .filter(|case| baseline.verdicts.get(case.case_id) == Some(&was) && case.passed != was)
            .map(|case| CaseDiff {
                case_id: case.case_id.to_owned(),
                title: case.case_id.to_owned(),
                critical: case.critical,
                baseline_url: format!("{}?case={}", baseline.url, case.case_id),
                candidate_url: format!("{candidate_url}?case={}", case.case_id),
            })
            .collect()
    };
    Diffs {
        regressions: changed(true),
        fixed: changed(false),
    }
}
