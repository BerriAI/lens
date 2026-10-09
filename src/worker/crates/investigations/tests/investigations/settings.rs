use super::support::*;
use chrono::{DateTime, TimeDelta, Utc};
use lens_contract::{
    investigations::{RunRequest, Scope},
    worker::LensSettings,
};
use lens_investigations::{analysis_checks, can_access, validate_run, validate_settings};
use rstest::rstest;
use serde_json::{Value, json};

#[rstest]
#[case::different_team(json!({"team_id":"alpha"}), json!({"team_id":"beta"}), false)]
#[case::global_target(json!({"team_id":"alpha"}), json!({"all_teams":true}), false)]
#[case::global_viewer(json!({"all_teams":true}), json!({"all_teams":true}), true)]
#[case::different_key(json!({"api_key_hash":"one"}), json!({"api_key_hash":"two"}), false)]
#[case::same_key(json!({"api_key_hash":"one"}), json!({"api_key_hash":"one"}), true)]
#[case::same_team(json!({"team_id":"alpha","api_key_hash":"one"}), json!({"team_id":"alpha","api_key_hash":"two"}), true)]
#[case::anonymous(json!({}), json!({}), true)]
fn scope_boundaries(#[case] viewer: Value, #[case] target: Value, #[case] allowed: bool) {
    assert_eq!(
        can_access(&decode::<Scope>(viewer), &decode(target)),
        allowed
    );
}

#[rstest]
#[case::valid(0.01, 0.01, true)]
#[case::maximum_percentage(100.0, 1000.0, true)]
#[case::zero_percentage(0.0, 100.0, false)]
#[case::above_percentage(100.01, 100.0, false)]
#[case::negative_percentage(-1.0, 100.0, false)]
#[case::infinite_percentage(f64::INFINITY, 100.0, false)]
#[case::nan_percentage(f64::NAN, 100.0, false)]
#[case::zero_budget(100.0, 0.0, false)]
#[case::negative_budget(100.0, -1.0, false)]
#[case::infinite_budget(100.0, f64::INFINITY, false)]
#[case::nan_budget(100.0, f64::NAN, false)]
fn finite_positive_selection_and_budget(
    settings: LensSettings,
    now: DateTime<Utc>,
    #[case] percentage: f64,
    #[case] budget: f64,
    #[case] valid: bool,
) {
    let candidate = LensSettings {
        sample_percent: percentage,
        monthly_budget: budget,
        ..settings
    };
    assert_eq!(validate_settings(&candidate, now).is_ok(), valid);
}

#[rstest]
#[case::empty("", json!([]), Some("Describe expected behavior or add an enabled check"))]
#[case::python_whitespace("\u{1c}\u{1d}\u{1e}\u{1f}\u{85}", json!([]), Some("Describe expected behavior or add an enabled check"))]
#[case::behavior("Expected behavior", json!([]), None)]
#[case::disabled("", json!([{"id":"one","instruction":"check","enabled":false}]), Some("Describe expected behavior or add an enabled check"))]
#[case::enabled("", json!([{"id":"one","instruction":"check"}]), None)]
#[case::duplicate("Behavior", json!([{"id":"one","instruction":"first"},{"id":"one","instruction":"other","enabled":false}]), Some("Each check must have a unique ID"))]
#[case::reserved("Behavior", json!([{"id":"expected_behavior","instruction":"check","enabled":false}]), Some("expected_behavior is reserved for the behavior description"))]
fn criteria_validation(
    settings: LensSettings,
    now: DateTime<Utc>,
    #[case] context: &str,
    #[case] checks: Value,
    #[case] message: Option<&str>,
) {
    let candidate = LensSettings {
        context: context.into(),
        checks: decode(checks),
        ..settings
    };
    assert_eq!(
        validate_settings(&candidate, now)
            .err()
            .map(|error| error.to_string())
            .as_deref(),
        message
    );
}

#[rstest]
#[case::no_behavior("", vec!["first", "last"])]
#[case::behavior(" Use citations ", vec!["expected_behavior", "first", "last"])]
fn enabled_checks_keep_order_after_expected_behavior(
    settings: LensSettings,
    #[case] context: &str,
    #[case] expected: Vec<&str>,
) {
    let candidate = LensSettings {
        context: context.into(),
        checks: decode(
            json!([{"id":"first","instruction":"First check"},{"id":"disabled","instruction":"Disabled check","enabled":false},{"id":"last","instruction":"Last check"}]),
        ),
        ..settings
    };
    let checks = analysis_checks(&candidate).unwrap();
    assert_eq!(
        checks
            .iter()
            .map(|check| check.id.as_str())
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        checks.iter().map(|check| check.enabled).collect::<Vec<_>>(),
        vec![true; expected.len()]
    );
    if !context.is_empty() {
        assert_eq!(
            checks[0].instruction.as_str(),
            "Identify deviations from the expected behavior described in context."
        );
    }
}

#[rstest]
#[case::large_valid(100_000, true)]
#[case::overflow(u64::MAX, false)]
fn calendar_limits(
    settings: LensSettings,
    now: DateTime<Utc>,
    #[case] value: u64,
    #[case] valid: bool,
    #[values(true, false)] lookback: bool,
) {
    let candidate = if lookback {
        LensSettings {
            lookback_hours: value.try_into().unwrap(),
            ..settings
        }
    } else {
        LensSettings {
            interval_minutes: value.try_into().unwrap(),
            ..settings
        }
    };
    assert_eq!(validate_settings(&candidate, now).is_ok(), valid);
}

#[rstest]
#[case::before_python_calendar("0001-01-01T00:00:00Z", false)]
#[case::after_python_calendar("9999-12-31T23:59:59Z", false)]
#[case::last_year_with_room("9999-12-30T00:00:00Z", true)]
fn calendar_range_matches_python_dates(
    settings: LensSettings,
    #[case] now: &str,
    #[case] valid: bool,
) {
    assert_eq!(
        validate_settings(&settings, now.parse().unwrap()).is_ok(),
        valid
    );
}

#[rstest]
#[case::invalid_settings(true)]
#[case::invalid_lookback(false)]
fn run_overrides_receive_the_same_validation(
    settings: LensSettings,
    now: DateTime<Utc>,
    #[case] settings_override: bool,
) {
    let request = if settings_override {
        RunRequest {
            settings: Some(LensSettings {
                checks: vec![],
                ..settings
            }),
            ..Default::default()
        }
    } else {
        RunRequest {
            lookback_hours: Some(u64::MAX.try_into().unwrap()),
            ..Default::default()
        }
    };
    assert!(validate_run(&request, now).is_err());
}

#[rstest]
#[case::start_missing(None, Some(1), Some("Choose both a start and an end time"))]
#[case::end_missing(Some(0), None, Some("Choose both a start and an end time"))]
#[case::equal(Some(0), Some(0), Some("Start time must be before end time"))]
#[case::reversed(Some(1), Some(0), Some("Start time must be before end time"))]
#[case::ordered(Some(0), Some(1), None)]
#[case::absent(None, None, None)]
fn run_window_validation(
    now: DateTime<Utc>,
    #[case] start: Option<i64>,
    #[case] end: Option<i64>,
    #[case] message: Option<&str>,
) {
    let request = RunRequest {
        start: start.map(|value| now + TimeDelta::hours(value)),
        end: end.map(|value| now + TimeDelta::hours(value)),
        ..Default::default()
    };
    assert_eq!(
        validate_run(&request, now)
            .err()
            .map(|error| error.to_string())
            .as_deref(),
        message
    );
}

#[rstest]
#[case::limit(200, true)]
#[case::too_long(201, false)]
fn agent_name_limits_count_characters(
    now: DateTime<Utc>,
    #[case] count: usize,
    #[case] valid: bool,
) {
    let request = RunRequest {
        agent_name: Some("é".repeat(count)),
        ..Default::default()
    };
    assert_eq!(validate_run(&request, now).is_ok(), valid);
}
