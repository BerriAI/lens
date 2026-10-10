use chrono::{DateTime, Utc};
use lens_contract::{
    investigations::{FindingImport, Lens},
    worker::{Evidence, FindingDraft, FindingDraftKind, LensSettingsSource},
};

use crate::{Error, merge_finding};

pub fn validate_import(lens: &Lens, request: &FindingImport) -> Result<(), Error> {
    if request.agent_name.trim().is_empty()
        || request.agent_name != lens.settings.agent_name
        || lens.settings.source == LensSettingsSource::Requests
    {
        return Err(Error::ImportScope);
    }
    if request.fingerprint.len() != 64
        || !request
            .fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        || request.title.len() > 160
        || request.description.len() > 4000
        || request.suggestion.len() > 4000
        || request.limitation.len() > 2000
        || !(1..=4).contains(&request.evidence.len())
        || request.evidence.iter().any(|source| {
            source.trace_id.is_empty()
                || source.trace_id.len() > 128
                || source.trace_ref.len() != 64
                || !source
                    .trace_ref
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
                || source.span_id.is_empty()
                || source.span_id.len() > 128
                || source.quote.trim().len() < 8
                || source.quote.len() > 240
        })
    {
        return Err(Error::InvalidImport);
    }
    Ok(())
}

pub fn import_finding(
    lens: &Lens,
    request: &FindingImport,
    evidence: Vec<Evidence>,
    now: DateTime<Utc>,
) -> Result<Lens, Error> {
    validate_import(lens, request)?;
    if evidence.len() != request.evidence.len()
        || evidence
            .iter()
            .zip(&request.evidence)
            .any(|(verified, source)| {
                verified.execution_id.is_empty()
                    || verified.span_id != source.span_id
                    || verified.quote != source.quote
            })
    {
        return Err(Error::InvalidImport);
    }
    let id = format!("agent-{}", request.fingerprint);
    let draft = FindingDraft {
        title: request.title.clone(),
        description: request.description.clone(),
        check_id: request.category.check_id().into(),
        check_ids: Vec::new(),
        kind: FindingDraftKind::Issue,
        priority: request.priority,
        suggestion: request.suggestion.clone(),
        limitation: request.limitation.clone(),
        brief: None,
        evidence,
        existing_finding_id: Some(id.clone()),
        merged_finding_ids: Vec::new(),
    };
    let finding = merge_finding(lens, &draft, lens.revision, now, &id, None, false)?;
    Ok(Lens {
        findings: std::iter::once(finding.clone())
            .chain(
                lens.findings
                    .iter()
                    .filter(|old| old.id != finding.id)
                    .cloned(),
            )
            .collect(),
        ..lens.clone()
    })
}
