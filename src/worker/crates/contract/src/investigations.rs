use crate::worker::{Finding, FindingStatus, Job, LensSettings, Review};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindingRun {
    pub finding_id: String,
    pub job_id: String,
}
use std::num::NonZeroU64;

pub mod public;
pub use public::Public;
mod import;
pub use import::{FindingCategory, FindingImport, FindingImported, FindingSource};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Scope {
    pub team_id: String,
    pub api_key_hash: String,
    pub all_teams: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BudgetReservation {
    pub id: String,
    pub job_id: String,
    #[schemars(range(min = 0))]
    pub amount: f64,
    pub month: String,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Lens {
    pub id: String,
    pub scope: Scope,
    #[serde(serialize_with = "public::serialize")]
    pub settings: LensSettings,
    #[serde(default = "initial_revision")]
    pub revision: i64,
    #[serde(default)]
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub next_run_at: DateTime<Utc>,
    #[serde(default)]
    pub last_scan_at: Option<DateTime<Utc>>,
    #[serde(default, serialize_with = "public::serialize")]
    pub jobs: Vec<Job>,
    #[serde(default, serialize_with = "public::serialize")]
    pub findings: Vec<Finding>,
    pub budget_month: String,
    #[serde(default)]
    pub spent: f64,
    #[serde(default)]
    pub reservations: Vec<BudgetReservation>,
    #[serde(default)]
    pub criteria_updated_at: Option<DateTime<Utc>>,
}

fn initial_revision() -> i64 {
    1
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Worker {
    #[serde(default, deserialize_with = "worker_analysis_key")]
    #[schemars(regex(pattern = "^[a-f0-9]{64}$"))]
    pub analysis_key_id: Option<String>,
    pub id: String,
    pub name: String,
    pub scope: Scope,
    pub last_seen: DateTime<Utc>,
    #[serde(default)]
    pub revoked: bool,
}

fn worker_analysis_key<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    let key = Option::<String>::deserialize(deserializer)?;
    if key.as_ref().is_some_and(|key| {
        key.len() != 64
            || !key
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    }) {
        return Err(serde::de::Error::custom(
            "analysis_key_id must contain 64 lowercase hexadecimal characters",
        ));
    }
    Ok(key)
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkerCreated {
    pub image: String,
    pub worker: Worker,
    pub token: String,
    #[serde(default)]
    pub managed: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LensList {
    pub lenses: Vec<Lens>,
    pub workers: Vec<Worker>,
    pub tracing_enabled: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct RunRequest {
    #[serde(serialize_with = "public::serialize")]
    pub settings: Option<LensSettings>,
    pub lookback_hours: Option<NonZeroU64>,
    pub start: Option<DateTime<Utc>>,
    pub end: Option<DateTime<Utc>>,
    #[schemars(length(max = 200))]
    pub agent_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewPage {
    #[serde(serialize_with = "public::serialize")]
    pub reviews: Vec<Review>,
    pub reviewed: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WatchSkipped {
    pub id: String,
    pub name: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WatchAllResult {
    pub watching: Vec<String>,
    #[serde(default)]
    pub skipped: Vec<WatchSkipped>,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindingUpdate {
    pub status: FindingStatus,
    #[serde(default)]
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceFindingCount {
    pub trace_id: String,
    pub trace_ref: String,
    #[serde(deserialize_with = "Option::deserialize")]
    pub finding_count: Option<u64>,
}
