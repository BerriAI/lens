mod support;

use chrono::{DateTime, TimeDelta, Utc};
use lens_contract::{
    investigations::{BudgetReservation, Lens},
    worker::{Job, Step},
};
use lens_inference::{
    BUDGET_LEASE, Error, renew_reservation, reserve_amount, reserve_attempt, settle_amount,
};
use lens_investigations::renew_budget;
use rstest::rstest;
use serde_json::{Value, json};
use support::{decode, job, lens, now, reservation, step};

#[rstest]
#[case::request_fits(90.0, 10.0, true)]
#[case::request_can_wait(89.0, 10.0, true)]
#[case::request_too_large(90.0, 10.001, false)]
fn admission_uses_remaining_monthly_budget(
    lens: Lens,
    reservation: BudgetReservation,
    now: DateTime<Utc>,
    #[case] spent: f64,
    #[case] amount: f64,
    #[case] accepted: bool,
) {
    let initial = Lens { spent, ..lens };
    let incoming = BudgetReservation {
        amount,
        ..reservation
    };
    let result = reserve_amount(&initial, &incoming, Some(now));
    if accepted {
        let admitted = result.unwrap();
        assert_eq!(admitted.spent, spent);
        assert_eq!(json!(admitted.reservations), json!([incoming]));
    } else {
        assert!(matches!(result, Err(Error::RequestBudget { .. })));
    }
}

#[rstest]
#[case::exhausted(
    100.0,
    "Monthly lens budget reached; increase it or wait for next month"
)]
#[case::overspent(
    102.0,
    "Monthly lens budget reached; increase it or wait for next month"
)]
#[case::insufficient(
    91.0,
    "This model request needs up to $10.000, but $9.000 remains in the investigation budget. Use a smaller deployment output allowance or increase the limit."
)]
fn admission_reports_actionable_budget_error(
    lens: Lens,
    reservation: BudgetReservation,
    #[case] spent: f64,
    #[case] diagnostic: &str,
) {
    assert_eq!(
        reserve_amount(&Lens { spent, ..lens }, &reservation, None)
            .unwrap_err()
            .to_string(),
        diagnostic
    );
}

#[rstest]
#[case::sufficient_room(80.0, true)]
#[case::exactly_full(90.0, true)]
#[case::pending_call(91.0, false)]
fn held_budget_waits_without_charging_or_reporting_exhaustion(
    lens: Lens,
    reservation: BudgetReservation,
    #[case] held: f64,
    #[case] accepted: bool,
) {
    let initial = Lens {
        reservations: vec![BudgetReservation {
            id: "other".into(),
            amount: held,
            ..reservation.clone()
        }],
        ..lens
    };
    let result = reserve_amount(&initial, &reservation, None).unwrap();
    assert_eq!(result.spent, initial.spent);
    assert_eq!(
        json!(result.reservations)
            .as_array()
            .unwrap()
            .contains(&json!(reservation)),
        accepted
    );
    assert_eq!(
        json!(result.reservations[0]),
        json!(initial.reservations[0])
    );
    if !accepted {
        assert_eq!(json!(result), json!(initial));
    }
}

#[rstest]
#[case::previous_month("2025-12", None, true, true)]
#[case::unexpired("2026-01", Some(1), true, false)]
#[case::at_expiry("2026-01", Some(0), true, true)]
#[case::expired("2026-01", Some(-1), true, true)]
#[case::permanent("2026-01", None, true, false)]
#[case::no_clock("2026-01", Some(-1), false, false)]
fn only_live_current_month_reservations_hold_budget(
    lens: Lens,
    reservation: BudgetReservation,
    now: DateTime<Utc>,
    #[case] month: &str,
    #[case] expiry_seconds: Option<i64>,
    #[case] use_clock: bool,
    #[case] accepted: bool,
) {
    let held = BudgetReservation {
        id: "held".into(),
        amount: 99.0,
        month: month.into(),
        expires_at: expiry_seconds.map(|seconds| now + TimeDelta::seconds(seconds)),
        ..reservation.clone()
    };
    let initial = Lens {
        reservations: vec![held],
        ..lens
    };
    let result = reserve_amount(&initial, &reservation, use_clock.then_some(now)).unwrap();
    assert_eq!(
        json!(result.reservations)
            .as_array()
            .unwrap()
            .contains(&json!(reservation)),
        accepted
    );
    assert_eq!(result.spent, 0.0);
}

#[rstest]
#[case::expired_yesterday(Some(-86401), true, false)]
#[case::retention_boundary(Some(-86400), true, false)]
#[case::recent_expiry(Some(-86399), true, true)]
#[case::no_expiry(None, true, true)]
#[case::no_clock(Some(-86401), false, true)]
fn admission_keeps_recent_expirations_for_late_settlement(
    lens: Lens,
    reservation: BudgetReservation,
    now: DateTime<Utc>,
    #[case] expiry_seconds: Option<i64>,
    #[case] use_clock: bool,
    #[case] retained: bool,
) {
    let stale = BudgetReservation {
        id: "stale".into(),
        amount: 1.0,
        expires_at: expiry_seconds.map(|seconds| now + TimeDelta::seconds(seconds)),
        ..reservation.clone()
    };
    let initial = Lens {
        reservations: vec![stale.clone()],
        ..lens
    };
    let result = reserve_amount(&initial, &reservation, use_clock.then_some(now)).unwrap();
    assert_eq!(
        json!(result.reservations)
            .as_array()
            .unwrap()
            .contains(&json!(stale)),
        retained
    );
    assert_eq!(json!(result.reservations.last()), json!(reservation));
}

#[rstest]
fn waiting_does_not_prune_reservations(
    lens: Lens,
    reservation: BudgetReservation,
    now: DateTime<Utc>,
) {
    let stale = BudgetReservation {
        id: "stale".into(),
        expires_at: Some(now - TimeDelta::days(2)),
        ..reservation.clone()
    };
    let full = BudgetReservation {
        id: "full".into(),
        amount: 100.0,
        ..reservation.clone()
    };
    let initial = Lens {
        reservations: vec![stale, full],
        ..lens
    };
    assert_eq!(
        json!(reserve_amount(&initial, &reservation, Some(now)).unwrap()),
        json!(initial)
    );
}

#[rstest]
fn settlement_releases_unused_allowance_and_is_idempotent(
    lens: Lens,
    reservation: BudgetReservation,
    step: Step,
) {
    let other = BudgetReservation {
        id: "other".into(),
        ..reservation.clone()
    };
    let initial = Lens {
        spent: 45.0,
        reservations: vec![reservation.clone(), other.clone()],
        ..lens
    };
    let settled = settle_amount(&initial, &reservation.id, 0.25, Some(&step));
    assert_eq!(settled.spent, 45.25);
    assert_eq!(json!(settled.reservations), json!([other]));
    assert_eq!(settled.jobs[0].cost, initial.jobs[0].cost + 0.25);
    assert_eq!(json!(settled.jobs[0].steps), json!([step]));
    assert_eq!(
        json!(settle_amount(&settled, &reservation.id, 0.25, None)),
        json!(settled)
    );
}

#[rstest]
#[case::recorded_job(true)]
#[case::archived_job(false)]
fn late_old_month_settlement_only_charges_the_job(
    lens: Lens,
    reservation: BudgetReservation,
    now: DateTime<Utc>,
    #[case] recorded_job: bool,
) {
    let old = BudgetReservation {
        month: "2025-12".into(),
        expires_at: Some(now),
        ..reservation
    };
    let initial = Lens {
        spent: 4.0,
        reservations: vec![old.clone()],
        jobs: if recorded_job {
            lens.jobs.clone()
        } else {
            vec![]
        },
        ..lens
    };
    let result = settle_amount(&initial, &old.id, 0.25, None);
    assert_eq!(result.spent, 4.0);
    assert!(result.reservations.is_empty());
    if recorded_job {
        assert_eq!(result.jobs[0].cost, initial.jobs[0].cost + 0.25);
        assert_eq!(json!(result.jobs[0].steps), json!(initial.jobs[0].steps));
    } else {
        assert!(result.jobs.is_empty());
    }
}

#[rstest]
fn settlement_finds_the_reserved_job_and_preserves_other_jobs(
    lens: Lens,
    job: Job,
    reservation: BudgetReservation,
) {
    let unrelated = Job {
        id: "other".into(),
        ..job
    };
    let initial = Lens {
        jobs: vec![unrelated.clone(), lens.jobs[0].clone()],
        reservations: vec![reservation.clone()],
        ..lens
    };
    let settled = settle_amount(&initial, &reservation.id, 0.5, None);
    assert_eq!(json!(settled.jobs[0]), json!(unrelated));
    assert_eq!(settled.jobs[1].cost, 2.5);
    assert_eq!(settled.spent, 0.5);
}

#[rstest]
fn missing_job_still_settles_monthly_spend(lens: Lens, reservation: BudgetReservation) {
    let initial = Lens {
        jobs: vec![],
        reservations: vec![reservation.clone()],
        spent: 1.0,
        ..lens
    };
    let result = settle_amount(&initial, &reservation.id, 0.25, None);
    assert_eq!(result.spent, 1.25);
    assert!(result.reservations.is_empty());
    assert!(result.jobs.is_empty());
}

#[rstest]
fn failed_call_releases_allowance_without_charging(lens: Lens, reservation: BudgetReservation) {
    let initial = Lens {
        reservations: vec![reservation.clone()],
        spent: 1.0,
        ..lens
    };
    let result = settle_amount(&initial, &reservation.id, 0.0, None);
    assert_eq!(result.spent, initial.spent);
    assert_eq!(json!(result.jobs), json!(initial.jobs));
    assert!(result.reservations.is_empty());
}

#[rstest]
#[case::missing(None, false)]
#[case::expired(Some(-1), true)]
#[case::exact_expiry(Some(0), true)]
fn renewal_never_resurrects_expired_or_released_allowance(
    lens: Lens,
    reservation: BudgetReservation,
    now: DateTime<Utc>,
    #[case] expiry_seconds: Option<i64>,
    #[case] present: bool,
) {
    let held = BudgetReservation {
        expires_at: expiry_seconds.map(|seconds| now + TimeDelta::seconds(seconds)),
        ..reservation
    };
    let initial = Lens {
        reservations: if present { vec![held.clone()] } else { vec![] },
        ..lens
    };
    let error = renew_reservation(&initial, &held.id, now).unwrap_err();
    assert!(matches!(error, Error::ReservationExpired));
    assert_eq!(
        error.to_string(),
        "Analysis budget reservation expired; retry the investigation"
    );
}

#[rstest]
#[case::leased(Some(1))]
#[case::legacy_permanent(None)]
fn renewal_extends_only_the_selected_reservation(
    lens: Lens,
    reservation: BudgetReservation,
    now: DateTime<Utc>,
    #[case] expiry_seconds: Option<i64>,
) {
    let held = BudgetReservation {
        expires_at: expiry_seconds.map(|seconds| now + TimeDelta::seconds(seconds)),
        ..reservation.clone()
    };
    let other = BudgetReservation {
        id: "other".into(),
        ..reservation
    };
    let initial = Lens {
        reservations: vec![other.clone(), held.clone()],
        ..lens
    };
    let result = renew_reservation(&initial, &held.id, now).unwrap();
    assert_eq!(
        json!(result.reservations),
        json!([
            other,
            BudgetReservation {
                expires_at: Some(now + TimeDelta::minutes(5)),
                ..held
            }
        ])
    );
    assert_eq!(BUDGET_LEASE, TimeDelta::minutes(5));
    assert_eq!(result.spent, initial.spent);
}

#[rstest]
#[case::selected_expired(0, 60, false)]
#[case::unrelated_expired(60, 0, true)]
fn renewal_checks_the_selected_reservations_lease(
    lens: Lens,
    reservation: BudgetReservation,
    now: DateTime<Utc>,
    #[case] selected_expiry: i64,
    #[case] other_expiry: i64,
    #[case] permitted: bool,
) {
    let selected = BudgetReservation {
        expires_at: Some(now + TimeDelta::seconds(selected_expiry)),
        ..reservation.clone()
    };
    let other = BudgetReservation {
        id: "other".into(),
        expires_at: Some(now + TimeDelta::seconds(other_expiry)),
        ..reservation
    };
    let initial = Lens {
        reservations: vec![other.clone(), selected.clone()],
        ..lens
    };
    let result = renew_reservation(&initial, &selected.id, now);
    if permitted {
        let renewed = result.unwrap();
        assert_eq!(json!(renewed.reservations[0]), json!(other));
        assert_eq!(renewed.reservations[1].expires_at, Some(now + BUDGET_LEASE));
    } else {
        assert!(matches!(result, Err(Error::ReservationExpired)));
    }
}

#[rstest]
fn another_live_reservation_does_not_allow_renewing_a_released_request(
    lens: Lens,
    reservation: BudgetReservation,
    now: DateTime<Utc>,
) {
    let initial = Lens {
        reservations: vec![reservation],
        ..lens
    };
    assert!(matches!(
        renew_reservation(&initial, "released", now),
        Err(Error::ReservationExpired)
    ));
}

#[rstest]
#[case::other_job(json!({"id":"other"}))]
#[case::queued(json!({"status":"queued"}))]
#[case::completed(json!({"status":"completed"}))]
#[case::cancelled(json!({"status":"cancelled"}))]
#[case::other_worker(json!({"worker_id":"other"}))]
#[case::unassigned(json!({"worker_id":null}))]
#[case::reclaimed(json!({"attempts":2}))]
#[case::no_lease(json!({"lease_until":null}))]
#[case::expired_lease(json!({"lease_until":"2026-01-15T11:59:59Z"}))]
#[case::lease_boundary(json!({"lease_until":"2026-01-15T12:00:00Z"}))]
fn admission_rechecks_the_job_attempt(
    lens: Lens,
    job: Job,
    reservation: BudgetReservation,
    now: DateTime<Utc>,
    #[case] changed: Value,
) {
    let mut value = json!(job);
    value
        .as_object_mut()
        .unwrap()
        .extend(changed.as_object().unwrap().clone());
    let current = Lens {
        jobs: vec![decode(value)],
        ..lens
    };
    let error = reserve_attempt(&current, &job, "worker", &reservation, now).unwrap_err();
    assert!(matches!(error, Error::JobReassigned));
    assert_eq!(error.to_string(), "Job was cancelled or reassigned");
    assert!(current.reservations.is_empty());
}

#[rstest]
fn current_owner_can_reserve_after_the_month_rolls_over(
    lens: Lens,
    job: Job,
    reservation: BudgetReservation,
    now: DateTime<Utc>,
) {
    let old = BudgetReservation {
        id: "old".into(),
        month: "2025-12".into(),
        amount: 100.0,
        ..reservation.clone()
    };
    let initial = Lens {
        budget_month: "2025-12".into(),
        spent: 100.0,
        reservations: vec![old.clone()],
        ..lens
    };
    let result = reserve_attempt(&initial, &job, "worker", &reservation, now).unwrap();
    assert_eq!(result.budget_month, "2026-01");
    assert_eq!(result.spent, 0.0);
    assert_eq!(json!(result.reservations), json!([old, reservation]));
    assert_eq!(json!(result.jobs), json!(initial.jobs));
}

#[rstest]
fn month_reset_preserves_pending_settlement_and_job_accounting(
    lens: Lens,
    reservation: BudgetReservation,
    now: DateTime<Utc>,
) {
    let initial = Lens {
        spent: 99.0,
        reservations: vec![reservation.clone()],
        ..lens
    };
    let renewed = renew_budget(&initial, now + TimeDelta::days(31));
    let settled = settle_amount(&renewed, &reservation.id, 0.5, None);
    assert_eq!(settled.spent, 0.0);
    assert_eq!(settled.budget_month, "2026-02");
    assert_eq!(settled.jobs[0].cost, 2.5);
    assert!(settled.reservations.is_empty());
}
