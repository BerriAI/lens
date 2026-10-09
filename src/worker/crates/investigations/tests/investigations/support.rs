use chrono::{DateTime, Utc};
use lens_contract::{
    investigations::{Lens, Worker},
    worker::{Activity, FindingDraft, Job, LensSettings, Review},
};
use lens_investigations::{QueueOptions, queue_job};
use rstest::fixture;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

pub fn decode<T: DeserializeOwned>(value: Value) -> T {
    serde_json::from_value(value).unwrap()
}

#[fixture]
pub fn now() -> DateTime<Utc> {
    "2026-01-15T00:00:00Z".parse().unwrap()
}

#[fixture]
pub fn settings() -> LensSettings {
    decode(
        json!({"name":"Research", "model":"analysis", "checks":[{"id":"retries", "instruction":"Find unrecovered retries"}]}),
    )
}

#[fixture]
pub fn lens() -> Lens {
    decode(
        json!({"id":"lens", "scope":{"team_id":"alpha"}, "settings":settings(), "created_at":now(), "next_run_at":now(), "budget_month":"2026-01"}),
    )
}

#[fixture]
pub fn worker() -> Worker {
    decode(json!({"id":"worker", "name":"Worker", "scope":{"team_id":"alpha"}, "last_seen":now()}))
}

#[fixture]
pub fn job() -> Job {
    queue_job(&lens(), now(), "job", QueueOptions::default())
        .unwrap()
        .jobs
        .remove(0)
}

pub fn review(index: usize) -> Review {
    decode(
        json!({"execution_id":format!("run-{index}"),"trace_id":"trace","agent":"support","name":"Task","model":"analysis","duration_ms":1,"at":now()}),
    )
}

pub fn activity(id: &str) -> Activity {
    decode(json!({"id":id,"phase":"review","label":"Review task","started_at":now()}))
}

pub fn draft(execution: &str) -> FindingDraft {
    decode(
        json!({"title":"Repeated failed searches","description":"The agent repeats the same failed search","check_id":"retries","evidence":[{"execution_id":execution,"span_id":"span","quote":"timeout"}]}),
    )
}
