use crate::Error;
use chrono::{DateTime, TimeDelta, Utc};
use lens_contract::{
    investigations::{BudgetReservation, Lens},
    worker::{Job, JobStatus, Step},
};
use lens_investigations::{add_step, current_job, renew_budget, replace_job};
use std::time::Duration;

pub const BUDGET_LEASE: TimeDelta = TimeDelta::minutes(5);
pub const BUDGET_RENEW_INTERVAL: Duration = Duration::from_secs(30);
pub const BUDGET_WAIT_TIMEOUT: Duration = Duration::from_secs(60);

pub fn reserve_amount(
    lens: &Lens,
    reservation: &BudgetReservation,
    now: Option<DateTime<Utc>>,
) -> Result<Lens, Error> {
    let available = lens.settings.monthly_budget - lens.spent;
    if reservation.amount > available {
        return Err(if available <= 0.0 {
            Error::MonthlyBudget
        } else {
            Error::RequestBudget {
                amount: reservation.amount,
                available,
            }
        });
    }
    let held: f64 = lens
        .reservations
        .iter()
        .filter(|item| {
            item.month == reservation.month
                && now.is_none_or(|now| item.expires_at.is_none_or(|expiry| expiry > now))
        })
        .map(|item| item.amount)
        .sum();
    if held + reservation.amount > available {
        return Ok(lens.clone());
    }
    Ok(Lens {
        reservations: lens
            .reservations
            .iter()
            .filter(|item| {
                now.is_none_or(|now| {
                    item.expires_at
                        .is_none_or(|expiry| expiry > now - TimeDelta::days(1))
                })
            })
            .cloned()
            .chain(std::iter::once(reservation.clone()))
            .collect(),
        ..lens.clone()
    })
}

pub fn reserve_attempt(
    lens: &Lens,
    job: &Job,
    worker_id: &str,
    reservation: &BudgetReservation,
    now: DateTime<Utc>,
) -> Result<Lens, Error> {
    let current = renew_budget(lens, now);
    let Some(active) = current_job(&current) else {
        return Err(Error::JobReassigned);
    };
    if active.id != job.id
        || active.status != JobStatus::Running
        || active.worker_id.as_deref() != Some(worker_id)
        || active.attempts != job.attempts
        || active.lease_until.is_none_or(|lease| lease <= now)
    {
        return Err(Error::JobReassigned);
    }
    reserve_amount(&current, reservation, Some(now))
}

pub fn settle_amount(lens: &Lens, reservation_id: &str, cost: f64, step: Option<&Step>) -> Lens {
    let Some(reservation) = lens
        .reservations
        .iter()
        .find(|item| item.id == reservation_id)
    else {
        return lens.clone();
    };
    let settled = Lens {
        spent: if lens.budget_month == reservation.month {
            lens.spent + cost
        } else {
            lens.spent
        },
        reservations: lens
            .reservations
            .iter()
            .filter(|item| item.id != reservation_id)
            .cloned()
            .collect(),
        ..lens.clone()
    };
    let Some(job) = lens.jobs.iter().find(|job| job.id == reservation.job_id) else {
        return settled;
    };
    let charged = Job {
        cost: job.cost + cost,
        ..job.clone()
    };
    let recorded = match step {
        Some(step) => add_step(&charged, step.clone()),
        None => charged,
    };
    replace_job(&settled, recorded)
}

pub fn renew_reservation(
    lens: &Lens,
    reservation_id: &str,
    now: DateTime<Utc>,
) -> Result<Lens, Error> {
    let reservation = lens
        .reservations
        .iter()
        .find(|item| item.id == reservation_id)
        .ok_or(Error::ReservationExpired)?;
    if reservation.expires_at.is_some_and(|expiry| expiry <= now) {
        return Err(Error::ReservationExpired);
    }
    Ok(Lens {
        reservations: lens
            .reservations
            .iter()
            .map(|item| {
                if item.id == reservation_id {
                    BudgetReservation {
                        expires_at: Some(now + BUDGET_LEASE),
                        ..item.clone()
                    }
                } else {
                    item.clone()
                }
            })
            .collect(),
        ..lens.clone()
    })
}
