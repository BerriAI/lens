use std::collections::BTreeMap;

use lens_contract::eval::CaseDiff;
use url::form_urlencoded::byte_serialize;

#[derive(Clone, Debug, PartialEq)]
pub struct Baseline {
    pub run_id: String,
    pub version: String,
    pub url: String,
    pub verdicts: BTreeMap<String, bool>,
}

pub fn is_critical(meta: &BTreeMap<String, String>) -> bool {
    meta.get("priority")
        .is_some_and(|priority| priority == "high")
}

pub(crate) struct CaseVerdict<'a> {
    pub case_id: &'a str,
    pub title: &'a str,
    pub critical: bool,
    pub passed: bool,
}

pub(crate) struct Diffs {
    pub regressions: Vec<CaseDiff>,
    pub fixed: Vec<CaseDiff>,
}

fn case_link(run_url: &str, case_id: &str) -> String {
    let separator = if run_url.contains('?') { '&' } else { '?' };
    let case_id: String = byte_serialize(case_id.as_bytes()).collect();
    format!("{run_url}{separator}case={case_id}")
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
                title: case.title.to_owned(),
                critical: case.critical,
                baseline_url: case_link(&baseline.url, case.case_id),
                candidate_url: case_link(candidate_url, case.case_id),
            })
            .collect()
    };
    Diffs {
        regressions: changed(true),
        fixed: changed(false),
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::case_link;

    #[rstest]
    #[case::plain("http://lens/runs/r1", "case-0", "http://lens/runs/r1?case=case-0")]
    #[case::existing_query(
        "http://lens/runs?id=r1",
        "case-0",
        "http://lens/runs?id=r1&case=case-0"
    )]
    #[case::reserved_characters(
        "http://lens/runs/r1",
        "a&b #c?",
        "http://lens/runs/r1?case=a%26b+%23c%3F"
    )]
    fn case_links_are_encoded(#[case] url: &str, #[case] case_id: &str, #[case] expected: &str) {
        assert_eq!(case_link(url, case_id), expected);
    }
}
