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

fn case_url(base: &str, case: &str) -> String {
    let Ok(mut url) = url::Url::parse(base) else {
        return base.to_owned();
    };
    let pairs: Vec<_> = url
        .query_pairs()
        .filter(|(key, _)| key != "eval_case")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    url.query_pairs_mut()
        .clear()
        .extend_pairs(pairs)
        .append_pair("eval_case", case);
    url.into()
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
                baseline_url: case_url(&baseline.url, case.case_id),
                candidate_url: case_url(candidate_url, case.case_id),
            })
            .collect()
    };
    Diffs {
        regressions: changed(true),
        fixed: changed(false),
    }
}
