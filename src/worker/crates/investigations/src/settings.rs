use crate::Error;
use chrono::{DateTime, Datelike, TimeDelta, Utc};
use lens_contract::{
    investigations::{RunRequest, Scope},
    worker::{Check, LensSettings},
};
use std::collections::BTreeSet;

pub fn can_access(viewer: &Scope, target: &Scope) -> bool {
    viewer.all_teams
        || (!target.all_teams
            && viewer.team_id == target.team_id
            && (!viewer.team_id.is_empty() || viewer.api_key_hash == target.api_key_hash))
}

pub(crate) fn trimmed(value: &str) -> &str {
    value.trim_matches(|character: char| {
        character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
    })
}

pub fn analysis_checks(settings: &LensSettings) -> Result<Vec<Check>, Error> {
    let behavior = if trimmed(&settings.context).is_empty() {
        None
    } else {
        Some(Check {
            id: "expected_behavior".try_into()?,
            instruction: "Identify deviations from the expected behavior described in context."
                .try_into()?,
            enabled: true,
        })
    };
    Ok(behavior
        .into_iter()
        .chain(
            settings
                .checks
                .iter()
                .filter(|check| check.enabled)
                .cloned(),
        )
        .collect())
}

pub fn validate_settings(settings: &LensSettings, now: DateTime<Utc>) -> Result<(), Error> {
    if !settings.sample_percent.is_finite()
        || settings.sample_percent <= 0.0
        || settings.sample_percent > 100.0
    {
        return Err(Error::SamplePercentage);
    }
    if !settings.monthly_budget.is_finite() || settings.monthly_budget <= 0.0 {
        return Err(Error::MonthlyBudget);
    }
    lookback_start(now, settings.lookback_hours.get())?;
    interval_end(now, settings.interval_minutes.get())?;
    let ids: BTreeSet<_> = settings.checks.iter().map(|check| &check.id).collect();
    if ids.len() != settings.checks.len() {
        return Err(Error::DuplicateCheck);
    }
    if trimmed(&settings.context).is_empty() && !settings.checks.iter().any(|check| check.enabled) {
        return Err(Error::MissingChecks);
    }
    if settings
        .checks
        .iter()
        .any(|check| check.id.as_str() == "expected_behavior")
    {
        return Err(Error::ReservedCheck);
    }
    Ok(())
}

pub fn validate_run(request: &RunRequest, now: DateTime<Utc>) -> Result<(), Error> {
    if let Some(settings) = &request.settings {
        validate_settings(settings, now)?;
    }
    if let Some(hours) = request.lookback_hours {
        lookback_start(now, hours.get())?;
    }
    if request
        .agent_name
        .as_ref()
        .is_some_and(|name| name.chars().count() > 200)
    {
        return Err(Error::AgentName);
    }
    match (request.start, request.end) {
        (Some(start), Some(end)) if start >= end => Err(Error::WindowOrder),
        (Some(_), None) | (None, Some(_)) => Err(Error::IncompleteWindow),
        _ => Ok(()),
    }
}

pub(crate) fn lookback_start(now: DateTime<Utc>, hours: u64) -> Result<DateTime<Utc>, Error> {
    let span = i64::try_from(hours).ok().and_then(TimeDelta::try_hours);
    calendar(
        span.and_then(|span| now.checked_sub_signed(span)),
        "Lookback",
    )
}

pub(crate) fn interval_end(now: DateTime<Utc>, minutes: u64) -> Result<DateTime<Utc>, Error> {
    let span = i64::try_from(minutes).ok().and_then(TimeDelta::try_minutes);
    calendar(
        span.and_then(|span| now.checked_add_signed(span)),
        "Interval",
    )
}

pub(crate) fn calendar(
    value: Option<DateTime<Utc>>,
    field: &'static str,
) -> Result<DateTime<Utc>, Error> {
    value
        .filter(|date| (1..=9999).contains(&date.year()))
        .ok_or(Error::CalendarRange(field))
}
