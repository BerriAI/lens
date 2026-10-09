use chrono::{DateTime, TimeDelta, Utc};
use lens_contract::{
    investigations::{BudgetReservation, Lens},
    worker::{Job, Step},
};
use rstest::fixture;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

pub fn decode<T: DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).unwrap()
}

#[fixture]
pub fn now() -> DateTime<Utc> {
    "2026-01-15T12:00:00Z".parse().unwrap()
}

#[fixture]
pub fn job() -> Job {
    decode(json!({
        "id":"job", "created_at":now(), "start":now() - TimeDelta::hours(1), "end":now(),
        "settings":{"name":"Research", "model":"analysis", "monthly_budget":100},
        "revision":1, "status":"running", "worker_id":"worker", "attempts":1,
        "lease_until":now() + TimeDelta::minutes(5), "cost":2
    }))
}

#[fixture]
pub fn lens() -> Lens {
    decode(json!({
        "id":"lens", "scope":{"team_id":"alpha"},
        "settings":{"name":"Research", "model":"analysis", "monthly_budget":100},
        "created_at":now(), "next_run_at":now(), "budget_month":"2026-01", "jobs":[job()]
    }))
}

#[fixture]
pub fn reservation() -> BudgetReservation {
    BudgetReservation {
        id: "request".into(),
        job_id: "job".into(),
        amount: 10.0,
        month: "2026-01".into(),
        expires_at: Some(now() + TimeDelta::minutes(5)),
    }
}

#[fixture]
pub fn step() -> Step {
    decode(json!({"at":now(), "kind":"model", "label":"Reviewed a run", "cost":0.25}))
}
