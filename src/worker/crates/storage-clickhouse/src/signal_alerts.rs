use chrono::{DateTime, NaiveDateTime, Utc};
use lens_contract::{
    signals::{SignalAttempt, SignalAttemptStatus, SignalConfig},
    worker::Execution,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    Error,
    state::{Change, ClickHouseState},
};

pub const SETTINGS: &str = "signal-alerts/settings";
pub const EVENTS: &str = "signal-alerts/events/";

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Settings {
    pub enabled: bool,
    pub signal_ids: Vec<String>,
    pub webhook: Value,
    pub generation: String,
    pub activated_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Event {
    pub generation: String,
    pub signal_id: String,
    pub name: String,
    pub score: f64,
    pub trace_id: String,
    pub trace_ref: String,
    pub created_at: DateTime<Utc>,
    pub status: DeliveryStatus,
    pub attempts: u8,
    pub retry_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    Pending,
    Sending,
    Delivered,
    Failed,
    Unconfirmed,
    Cancelled,
}

pub async fn settings(store: &ClickHouseState) -> Result<Settings, Error> {
    let value = store.read(SETTINGS).await?.value;
    if value.is_null() {
        Ok(Settings::default())
    } else {
        serde_json::from_value(value).map_err(|_| Error::InvalidResponse)
    }
}

fn eligible(settings: &Settings, execution: &Execution, attempt: &SignalAttempt) -> bool {
    settings.enabled
        && attempt.status == SignalAttemptStatus::Classified
        && settings.activated_at.is_some_and(|activated| {
            DateTime::parse_from_rfc3339(&execution.start_time)
                .map(|value| value.with_timezone(&Utc))
                .or_else(|_| {
                    NaiveDateTime::parse_from_str(&execution.start_time, "%Y-%m-%d %H:%M:%S%.f")
                        .map(|value| value.and_utc())
                })
                .or_else(|_| {
                    NaiveDateTime::parse_from_str(&execution.start_time, "%Y-%m-%dT%H:%M:%S%.f")
                        .map(|value| value.and_utc())
                })
                .is_ok_and(|started| started >= activated)
        })
}

pub async fn enqueue(
    store: &ClickHouseState,
    execution: &Execution,
    config: &SignalConfig,
    attempt: &SignalAttempt,
    now: DateTime<Utc>,
) -> Result<Vec<Change>, Error> {
    if attempt.status != SignalAttemptStatus::Classified {
        return Ok(Vec::new());
    }
    let settings = settings(store).await?;
    if !eligible(&settings, execution, attempt) {
        return Ok(Vec::new());
    }
    let mut changes = Vec::new();
    for signal in &config.signals {
        let Some(score) = attempt
            .scores
            .get(&signal.id)
            .filter(|score| **score >= config.threshold)
        else {
            continue;
        };
        if !settings.signal_ids.contains(&signal.id) {
            continue;
        }
        let key = format!(
            "{EVENTS}{:x}",
            Sha256::digest(
                serde_json::to_vec(&(&execution.trace_id, &execution.trace_ref, &signal.id))
                    .map_err(|_| Error::InvalidState)?
            )
        );
        let previous = store.read(&key).await?;
        if !previous.value.is_null() {
            continue;
        }
        let event = Event {
            generation: settings.generation.clone(),
            signal_id: signal.id.clone(),
            name: signal.name.clone(),
            score: *score,
            trace_id: execution.trace_id.clone(),
            trace_ref: execution.trace_ref.clone(),
            created_at: now,
            status: DeliveryStatus::Pending,
            attempts: 0,
            retry_at: now,
        };
        changes.push(Change {
            previous,
            value: serde_json::to_value(event).map_err(|_| Error::InvalidState)?,
        });
    }
    Ok(changes)
}
