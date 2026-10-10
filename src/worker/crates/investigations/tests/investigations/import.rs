use super::support::{decode, lens, now};
use chrono::{DateTime, TimeDelta, Utc};
use lens_contract::{
    investigations::{FindingImport, Lens},
    worker::{Evidence, FindingStatus, LensSettings},
};
use lens_investigations::{Error, import_finding, validate_import};
use rstest::{fixture, rstest};
use serde_json::{Value, json};

#[fixture]
fn target(lens: Lens) -> Lens {
    Lens {
        settings: LensSettings {
            agent_name: "test-agent".into(),
            ..lens.settings.clone()
        },
        ..lens
    }
}

#[fixture]
fn request() -> FindingImport {
    decode(json!({
        "fingerprint": "a".repeat(64), "agent_name": "test-agent",
        "category": "Reliability", "priority": "high", "title": "Search returned an error",
        "description": "The recorded search failed to complete the task",
        "suggestion": "Test bounded retries", "limitation": "Proposed change, not measured",
        "evidence": [{"trace_id": "trace", "trace_ref": "B".repeat(64),
            "span_id": "span", "quote": "Recorded search failure"}]
    }))
}

#[fixture]
fn evidence() -> Vec<Evidence> {
    vec![decode(
        json!({"execution_id": "scoped-execution", "span_id": "span", "quote": "Recorded search failure"}),
    )]
}

#[rstest]
fn imports_merge_without_losing_feedback_or_counting_retries(
    target: Lens,
    request: FindingImport,
    evidence: Vec<Evidence>,
    now: DateTime<Utc>,
) {
    let first = import_finding(&target, &request, evidence.clone(), now).unwrap();
    let saved = Lens {
        findings: vec![lens_contract::worker::Finding {
            status: FindingStatus::Resolved,
            reason: "Verified by owner".into(),
            ..first.findings[0].clone()
        }],
        ..first
    };
    let replay = import_finding(&saved, &request, evidence, now + TimeDelta::hours(1)).unwrap();
    assert_eq!(replay.findings.len(), 1);
    let finding = &replay.findings[0];
    assert_eq!(finding.id, format!("agent-{}", request.fingerprint));
    assert_eq!(finding.status, FindingStatus::Resolved);
    assert_eq!(finding.reason, "Verified by owner");
    assert_eq!(finding.first_seen, now);
    assert_eq!(finding.last_seen, now);
    assert_eq!(finding.occurrences, vec!["scoped-execution"]);
    assert_eq!(finding.evidence.len(), 1);
    assert_eq!(finding.check_id, "reliability");
    assert_eq!(finding.description.as_str(), request.description.as_str());
    assert_eq!(finding.suggestion, request.suggestion);
    assert_eq!(finding.limitation, request.limitation);
    assert!(finding.investigation_runs.is_empty());
    assert!(replay.jobs.is_empty());
}

#[rstest]
#[case::agent("/agent_name", json!("another-agent"), true)]
#[case::fingerprint("/fingerprint", json!("arbitrary-id"), false)]
#[case::unscoped_trace("/evidence/0/trace_ref", json!(""), false)]
#[case::missing_span("/evidence/0/span_id", json!(""), false)]
#[case::empty_evidence("/evidence", json!([]), false)]
#[case::long_quote("/evidence/0/quote", json!("x".repeat(241)), false)]
fn invalid_import_cannot_create_a_finding(
    target: Lens,
    request: FindingImport,
    #[case] path: &str,
    #[case] value: Value,
    #[case] scope_error: bool,
) {
    let mut body = json!(request);
    *body.pointer_mut(path).unwrap() = value;
    let invalid: FindingImport = decode(body);
    let result = validate_import(&target, &invalid);
    assert!(matches!(result, Err(Error::ImportScope)) == scope_error);
    assert!(result.is_err());
    assert!(target.findings.is_empty());
}

#[rstest]
#[case::missing(vec![])]
#[case::changed_quote(vec![decode(json!({"execution_id":"scoped-execution","span_id":"span","quote":"Forged search failure"}))])]
#[case::changed_span(vec![decode(json!({"execution_id":"scoped-execution","span_id":"another-span","quote":"Recorded search failure"}))])]
fn unverified_evidence_cannot_be_imported(
    target: Lens,
    request: FindingImport,
    now: DateTime<Utc>,
    #[case] verified: Vec<Evidence>,
) {
    assert!(matches!(
        import_finding(&target, &request, verified, now),
        Err(Error::InvalidImport)
    ));
}

#[rstest]
#[case::title_boundary("/title", "界".repeat(160), true)]
#[case::title_overflow("/title", "界".repeat(161), false)]
#[case::quote_boundary("/evidence/0/quote", "界".repeat(240), true)]
#[case::quote_overflow("/evidence/0/quote", "界".repeat(241), false)]
#[case::short_quote("/evidence/0/quote", "界".repeat(7), false)]
#[case::description_boundary("/description", "界".repeat(4000), true)]
#[case::description_overflow("/description", "界".repeat(4001), false)]
#[case::suggestion_boundary("/suggestion", "界".repeat(4000), true)]
#[case::suggestion_overflow("/suggestion", "界".repeat(4001), false)]
#[case::limitation_boundary("/limitation", "界".repeat(2000), true)]
#[case::limitation_overflow("/limitation", "界".repeat(2001), false)]
#[case::emoji_title_boundary("/title", "🔎".repeat(80), true)]
#[case::emoji_title_overflow("/title", "🔎".repeat(81), false)]
#[case::emoji_quote_boundary("/evidence/0/quote", "🔎".repeat(120), true)]
#[case::emoji_quote_overflow("/evidence/0/quote", "🔎".repeat(121), false)]
#[case::emoji_quote_minimum("/evidence/0/quote", "🔎".repeat(4), true)]
#[case::emoji_quote_too_short("/evidence/0/quote", "🔎".repeat(3), false)]
fn imported_text_limits_match_javascript_strings(
    target: Lens,
    request: FindingImport,
    #[case] path: &str,
    #[case] text: String,
    #[case] valid: bool,
) {
    let mut body = json!(request);
    *body.pointer_mut(path).unwrap() = json!(text);
    let request: FindingImport = decode(body);
    assert_eq!(validate_import(&target, &request).is_ok(), valid);
}
