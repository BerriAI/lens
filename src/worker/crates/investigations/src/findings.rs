use crate::Error;
use chrono::{DateTime, Utc};
use lens_contract::{
    investigations::Lens,
    worker::{
        Evidence, EvidenceRole, Finding, FindingDraft, FindingDraftKind, FindingDraftPriority,
        FindingKind, FindingPriority, FindingStatus,
    },
};
use std::collections::BTreeSet;
use unicode_casefold::UnicodeCaseFold;

pub fn merge_finding(
    lens: &Lens,
    draft: &FindingDraft,
    revision: i64,
    now: DateTime<Utc>,
    new_id: &str,
    job_id: Option<&str>,
    match_titles: bool,
) -> Result<Finding, Error> {
    let identities: BTreeSet<_> = draft
        .existing_finding_id
        .iter()
        .chain(&draft.merged_finding_ids)
        .map(String::as_str)
        .collect();
    let mut matches: Vec<_> = lens
        .findings
        .iter()
        .filter(|finding| {
            finding.kind == kind(draft.kind)
                && (identities.contains(finding.id.as_str())
                    || finding
                        .merged_finding_ids
                        .iter()
                        .any(|id| identities.contains(id.as_str()))
                    || (match_titles
                        && draft.existing_finding_id.is_none()
                        && finding.title.case_fold().eq(draft.title.case_fold())
                        && finding.check_id == draft.check_id))
        })
        .collect();
    matches.sort_by(|left, right| (left.first_seen, &left.id).cmp(&(right.first_seen, &right.id)));
    let previous: Vec<_> = matches.first().map_or_else(Vec::new, |first| {
        matches
            .iter()
            .copied()
            .filter(|finding| (finding.status, &finding.reason) == (first.status, &first.reason))
            .collect()
    });
    let first = previous.first().copied();
    let occurrences = occurrences(&draft.evidence);
    let prior: BTreeSet<_> = previous
        .iter()
        .flat_map(|finding| finding.occurrences.iter().cloned())
        .collect();
    let new_occurrence = !occurrences.is_subset(&prior);
    let check_ids = previous
        .iter()
        .flat_map(|finding| std::iter::once(&finding.check_id).chain(&finding.check_ids))
        .chain(std::iter::once(&draft.check_id).chain(&draft.check_ids))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let evidence = unique_evidence(
        previous
            .iter()
            .flat_map(|finding| &finding.evidence)
            .chain(&draft.evidence),
    );
    let merged_finding_ids = previous
        .iter()
        .flat_map(|finding| std::iter::once(&finding.id).chain(&finding.merged_finding_ids))
        .filter(|id| first.is_none_or(|first| *id != &first.id))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut seen_runs = BTreeSet::new();
    let investigation_runs = previous
        .iter()
        .flat_map(|finding| finding.investigation_runs.iter().map(String::as_str))
        .chain(job_id.filter(|id| !id.is_empty() && new_occurrence))
        .filter(|id| seen_runs.insert(*id))
        .map(str::to_owned)
        .collect();
    Ok(Finding {
        id: first.map_or(new_id, |finding| &finding.id).into(),
        title: draft.title.as_str().try_into()?,
        description: draft.description.as_str().try_into()?,
        check_id: draft.check_id.clone(),
        kind: kind(draft.kind),
        priority: priority(draft.priority),
        suggestion: draft.suggestion.clone(),
        limitation: draft.limitation.clone(),
        brief: draft
            .brief
            .clone()
            .or_else(|| first.and_then(|finding| finding.brief.clone())),
        evidence,
        existing_finding_id: draft.existing_finding_id.clone(),
        check_ids,
        merged_finding_ids,
        status: match first {
            Some(finding) if finding.status != FindingStatus::Resolved || !new_occurrence => {
                finding.status
            }
            _ => FindingStatus::Open,
        },
        reason: first.map_or_else(String::new, |finding| finding.reason.clone()),
        first_seen: first.map_or(now, |finding| finding.first_seen),
        last_seen: if new_occurrence {
            now
        } else {
            previous
                .iter()
                .map(|finding| finding.last_seen)
                .max()
                .unwrap_or(now)
        },
        occurrences: prior.union(&occurrences).cloned().collect(),
        revision,
        investigation_runs,
    })
}

pub fn snapshot_finding(
    lens: &Lens,
    draft: &FindingDraft,
    revision: i64,
    now: DateTime<Utc>,
    new_id: &str,
) -> Result<Finding, Error> {
    let merged = merge_finding(lens, draft, revision, now, new_id, None, true)?;
    Ok(Finding {
        brief: draft.brief.clone(),
        evidence: draft.evidence.clone(),
        check_ids: draft.check_ids.clone(),
        merged_finding_ids: draft.merged_finding_ids.clone(),
        first_seen: now,
        last_seen: now,
        occurrences: occurrences(&draft.evidence).into_iter().collect(),
        ..merged
    })
}

fn kind(value: FindingDraftKind) -> FindingKind {
    match value {
        FindingDraftKind::Issue => FindingKind::Issue,
        FindingDraftKind::Pattern => FindingKind::Pattern,
    }
}

fn priority(value: FindingDraftPriority) -> FindingPriority {
    match value {
        FindingDraftPriority::High => FindingPriority::High,
        FindingDraftPriority::Medium => FindingPriority::Medium,
        FindingDraftPriority::Low => FindingPriority::Low,
    }
}

fn occurrences(evidence: &[Evidence]) -> BTreeSet<String> {
    evidence
        .iter()
        .filter(|quote| quote.role == EvidenceRole::Support)
        .map(|quote| quote.execution_id.clone())
        .collect()
}

fn unique_evidence<'a>(evidence: impl Iterator<Item = &'a Evidence>) -> Vec<Evidence> {
    let mut seen = BTreeSet::new();
    evidence
        .filter(|quote| {
            seen.insert((
                &quote.execution_id,
                &quote.span_id,
                &quote.quote,
                quote.role,
            ))
        })
        .cloned()
        .collect()
}
