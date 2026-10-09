use super::support::*;
use chrono::{DateTime, TimeDelta, Utc};
use lens_contract::{
    investigations::Lens,
    worker::{Finding, FindingDraft, FindingDraftKind, FindingDraftPriority, FindingStatus},
};
use lens_investigations::{merge_finding, snapshot_finding};
use rstest::rstest;
use serde_json::json;

fn saved(lens: &Lens, now: DateTime<Utc>, execution: &str, id: &str) -> Finding {
    merge_finding(lens, &draft(execution), 1, now, id, Some("first-run"), true).unwrap()
}

#[rstest]
fn new_finding_preserves_all_draft_fields(
    lens: Lens,
    now: DateTime<Utc>,
    #[values(FindingDraftKind::Issue, FindingDraftKind::Pattern)] kind: FindingDraftKind,
    #[values(
        FindingDraftPriority::High,
        FindingDraftPriority::Medium,
        FindingDraftPriority::Low
    )]
    priority: FindingDraftPriority,
) {
    let draft = FindingDraft {
        kind,
        priority,
        suggestion: "Try recovery".into(),
        limitation: "Partial response".into(),
        existing_finding_id: Some("missing".into()),
        check_ids: vec!["secondary".into()],
        ..draft("run")
    };
    let finding = merge_finding(&lens, &draft, 7, now, "generated", Some("job"), true).unwrap();
    assert_eq!(finding.id, "generated");
    assert_eq!(finding.kind.to_string(), kind.to_string());
    assert_eq!(finding.priority.to_string(), priority.to_string());
    assert_eq!(finding.title.as_str(), draft.title.as_str());
    assert_eq!(finding.description.as_str(), draft.description.as_str());
    assert_eq!(
        (
            &finding.suggestion,
            &finding.limitation,
            &finding.existing_finding_id
        ),
        (
            &draft.suggestion,
            &draft.limitation,
            &draft.existing_finding_id
        )
    );
    assert_eq!(json!(finding.evidence), json!(draft.evidence));
    assert_eq!(finding.check_ids, vec!["retries", "secondary"]);
    assert_eq!(finding.occurrences, vec!["run"]);
    assert_eq!(finding.investigation_runs, vec!["job"]);
    assert_eq!(
        (finding.revision, finding.first_seen, finding.last_seen),
        (7, now, now)
    );
    assert_eq!(finding.status, FindingStatus::Open);
    assert_eq!(finding.reason, "");
    assert!(finding.merged_finding_ids.is_empty());
}

#[rstest]
#[case::replay(FindingStatus::Resolved, "old", false, FindingStatus::Resolved)]
#[case::new_support(FindingStatus::Resolved, "new", false, FindingStatus::Open)]
#[case::counterexample(FindingStatus::Resolved, "new", true, FindingStatus::Resolved)]
#[case::dismissed(FindingStatus::Dismissed, "new", false, FindingStatus::Dismissed)]
#[case::open(FindingStatus::Open, "new", false, FindingStatus::Open)]
fn feedback_survives_replay_and_only_new_support_reopens(
    lens: Lens,
    now: DateTime<Utc>,
    #[case] status: FindingStatus,
    #[case] execution: &str,
    #[case] counterexample: bool,
    #[case] expected: FindingStatus,
) {
    let prior = Finding {
        status,
        reason: "Reviewed".into(),
        ..saved(&lens, now, "old", "existing")
    };
    let stored = Lens {
        findings: vec![prior.clone()],
        ..lens
    };
    let new = if counterexample {
        FindingDraft {
            evidence: decode(
                json!([{"execution_id":execution,"span_id":"span","quote":"Recovered","role":"counterexample"}]),
            ),
            ..draft(execution)
        }
    } else {
        draft(execution)
    };
    let merged = merge_finding(
        &stored,
        &new,
        2,
        now + TimeDelta::days(1),
        "unused",
        Some("new-run"),
        true,
    )
    .unwrap();
    assert_eq!(merged.id, "existing");
    assert_eq!(merged.status, expected);
    assert_eq!(merged.reason, "Reviewed");
    assert_eq!(merged.first_seen, now);
    let new_occurrence = execution == "new" && !counterexample;
    assert_eq!(
        merged.last_seen,
        if new_occurrence {
            now + TimeDelta::days(1)
        } else {
            now
        }
    );
    assert_eq!(
        merged.occurrences,
        if new_occurrence {
            vec!["new", "old"]
        } else {
            vec!["old"]
        }
    );
    assert_eq!(
        merged.investigation_runs,
        if new_occurrence {
            vec!["first-run", "new-run"]
        } else {
            vec!["first-run"]
        }
    );
}

#[rstest]
#[case::same_title(true, None, "retries", true)]
#[case::title_disabled(false, None, "retries", false)]
#[case::different_check(true, None, "other", false)]
#[case::explicit_cross_check(true, Some("existing"), "other", true)]
#[case::missing_explicit_id(true, Some("missing"), "retries", false)]
fn finding_identity_rules(
    lens: Lens,
    now: DateTime<Utc>,
    #[case] titles: bool,
    #[case] existing: Option<&str>,
    #[case] check: &str,
    #[case] reused: bool,
) {
    let old = Finding {
        status: FindingStatus::Dismissed,
        reason: "Accepted".into(),
        ..saved(&lens, now, "old", "existing")
    };
    let stored = Lens {
        findings: vec![old],
        ..lens
    };
    let new = FindingDraft {
        check_id: check.into(),
        existing_finding_id: existing.map(str::to_owned),
        ..draft("new")
    };
    let merged = merge_finding(&stored, &new, 2, now, "generated", None, titles).unwrap();
    assert_eq!(merged.id, if reused { "existing" } else { "generated" });
    assert_eq!(
        merged.status,
        if reused {
            FindingStatus::Dismissed
        } else {
            FindingStatus::Open
        }
    );
    assert_eq!(merged.reason, if reused { "Accepted" } else { "" });
}

#[rstest]
#[case::by_title(false)]
#[case::explicit(true)]
fn issues_and_patterns_have_independent_feedback(
    lens: Lens,
    now: DateTime<Utc>,
    #[case] explicit: bool,
) {
    let old = Finding {
        status: FindingStatus::Dismissed,
        ..saved(&lens, now, "old", "issue")
    };
    let stored = Lens {
        findings: vec![old],
        ..lens
    };
    let new = FindingDraft {
        kind: FindingDraftKind::Pattern,
        existing_finding_id: explicit.then(|| "issue".into()),
        ..draft("new")
    };
    let merged = merge_finding(&stored, &new, 2, now, "pattern", None, true).unwrap();
    assert_eq!(merged.id, "pattern");
    assert_eq!(merged.status, FindingStatus::Open);
    assert_eq!(merged.occurrences, vec!["new"]);
}

#[rstest]
fn title_matching_uses_unicode_casefold(lens: Lens, now: DateTime<Utc>) {
    let old = Finding {
        title: "Straße".try_into().unwrap(),
        ..saved(&lens, now, "old", "existing")
    };
    let stored = Lens {
        findings: vec![old],
        ..lens
    };
    let new = FindingDraft {
        title: "STRASSE".try_into().unwrap(),
        ..draft("new")
    };
    assert_eq!(
        merge_finding(&stored, &new, 2, now, "new", None, true)
            .unwrap()
            .id,
        "existing"
    );
}

#[rstest]
fn merges_follow_aliases_and_oldest_identity_with_consistent_feedback(
    lens: Lens,
    now: DateTime<Utc>,
) {
    let oldest = Finding {
        merged_finding_ids: vec!["alias".into()],
        check_ids: vec!["extra".into()],
        ..saved(&lens, now, "a", "oldest")
    };
    let second = Finding {
        check_id: "second".into(),
        ..saved(&lens, now + TimeDelta::minutes(1), "b", "second")
    };
    let feedback = Finding {
        status: FindingStatus::Resolved,
        reason: "Fixed".into(),
        ..saved(&lens, now + TimeDelta::minutes(2), "ignored", "feedback")
    };
    let stored = Lens {
        findings: vec![feedback, second.clone(), oldest.clone()],
        ..lens
    };
    let new = FindingDraft {
        title: "Updated wording".try_into().unwrap(),
        check_id: "new".into(),
        existing_finding_id: Some("alias".into()),
        merged_finding_ids: vec!["second".into(), "feedback".into()],
        evidence: vec![oldest.evidence[0].clone(), draft("c").evidence[0].clone()],
        ..draft("c")
    };
    let merged = merge_finding(
        &stored,
        &new,
        3,
        now + TimeDelta::hours(1),
        "unused",
        Some("new-run"),
        false,
    )
    .unwrap();
    assert_eq!(merged.id, "oldest");
    assert_eq!(merged.merged_finding_ids, vec!["alias", "second"]);
    assert_eq!(merged.occurrences, vec!["a", "b", "c"]);
    assert_eq!(merged.check_ids, vec!["extra", "new", "retries", "second"]);
    assert_eq!(merged.investigation_runs, vec!["first-run", "new-run"]);
    assert_eq!(
        merged
            .evidence
            .iter()
            .map(|evidence| evidence.execution_id.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b", "c"]
    );
    assert_eq!(merged.status, FindingStatus::Open);
    assert_eq!(merged.reason, "");
}

#[rstest]
fn equal_age_identities_sort_by_id_and_keep_max_seen_on_replay(lens: Lens, now: DateTime<Utc>) {
    let a = saved(&lens, now, "a", "a");
    let z = Finding {
        last_seen: now + TimeDelta::days(2),
        ..saved(&lens, now, "z", "z")
    };
    let stored = Lens {
        findings: vec![z, a],
        ..lens
    };
    let merged = merge_finding(
        &stored,
        &draft("a"),
        3,
        now + TimeDelta::days(3),
        "unused",
        Some("new-run"),
        true,
    )
    .unwrap();
    assert_eq!(merged.id, "a");
    assert_eq!(merged.last_seen, now + TimeDelta::days(2));
    assert_eq!(merged.occurrences, vec!["a", "z"]);
    assert_eq!(merged.merged_finding_ids, vec!["z"]);
    assert_eq!(merged.investigation_runs, vec!["first-run"]);
}

#[rstest]
fn explicit_match_does_not_absorb_same_title_with_changed_feedback(lens: Lens, now: DateTime<Utc>) {
    let original = saved(&lens, now, "a", "original");
    let other = Finding {
        status: FindingStatus::Dismissed,
        reason: "Intentional".into(),
        ..saved(&lens, now, "b", "other")
    };
    let stored = Lens {
        findings: vec![original, other],
        ..lens
    };
    let new = FindingDraft {
        existing_finding_id: Some("original".into()),
        ..draft("c")
    };
    let merged = merge_finding(&stored, &new, 2, now, "unused", None, true).unwrap();
    assert_eq!(merged.occurrences, vec!["a", "c"]);
    assert!(merged.merged_finding_ids.is_empty());
    assert_eq!(merged.status, FindingStatus::Open);
}

fn brief(problem: &str) -> lens_contract::worker::IssueBrief {
    decode(
        json!({"problem":problem,"user_goal":"Open a pull request","what_happened":"Repository access was missing","test_cases":[{"input":"Fix a typo","expected":"A PR URL"}]}),
    )
}

#[rstest]
#[case::keep(None, "Missing repository access")]
#[case::replace(Some("Provider token expired"), "Provider token expired")]
fn issue_briefs_persist_until_replaced(
    lens: Lens,
    now: DateTime<Utc>,
    #[case] replacement: Option<&str>,
    #[case] expected: &str,
) {
    let old = Finding {
        brief: Some(brief("Missing repository access")),
        ..saved(&lens, now, "old", "id")
    };
    let stored = Lens {
        findings: vec![old],
        ..lens
    };
    let new = FindingDraft {
        brief: replacement.map(brief),
        ..draft("new")
    };
    assert_eq!(
        merge_finding(&stored, &new, 2, now, "unused", None, true)
            .unwrap()
            .brief
            .unwrap()
            .problem
            .as_str(),
        expected
    );
}

#[rstest]
#[case::new_occurrence("new")]
#[case::replayed_occurrence("old")]
fn run_snapshot_preserves_feedback_but_contains_only_current_evidence(
    lens: Lens,
    now: DateTime<Utc>,
    #[case] execution: &str,
) {
    let old = Finding {
        status: FindingStatus::Dismissed,
        reason: "Expected recovery".into(),
        brief: Some(brief("Missing repository access")),
        merged_finding_ids: vec!["historical-alias".into()],
        ..saved(&lens, now, "old", "existing")
    };
    let stored = Lens {
        findings: vec![old],
        ..lens
    };
    let new = FindingDraft {
        title: "Updated wording".try_into().unwrap(),
        existing_finding_id: Some("existing".into()),
        ..draft(execution)
    };
    let at = now + TimeDelta::days(1);
    let snapshot = snapshot_finding(&stored, &new, 3, at, "unused").unwrap();
    assert_eq!(snapshot.id, "existing");
    assert_eq!(snapshot.status, FindingStatus::Dismissed);
    assert_eq!(snapshot.reason, "Expected recovery");
    assert_eq!(snapshot.occurrences, vec![execution]);
    assert_eq!(json!(snapshot.evidence), json!(new.evidence));
    assert!(snapshot.brief.is_none());
    assert!(snapshot.check_ids.is_empty() && snapshot.merged_finding_ids.is_empty());
    assert_eq!(
        (snapshot.revision, snapshot.first_seen, snapshot.last_seen),
        (3, at, at)
    );
    assert_eq!(snapshot.investigation_runs, vec!["first-run"]);
}

#[rstest]
#[case::none(None)]
#[case::empty(Some(""))]
fn absent_run_ids_are_not_recorded(lens: Lens, now: DateTime<Utc>, #[case] id: Option<&str>) {
    assert!(
        merge_finding(&lens, &draft("new"), 1, now, "new", id, true)
            .unwrap()
            .investigation_runs
            .is_empty()
    );
}
