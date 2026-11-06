use std::collections::BTreeSet;

use chrono::{DateTime, TimeDelta, Utc};
use lens_contract::{
    feedback::TraceIdentity,
    signals::{SignalAttempt, SignalAttemptStatus, SignalConfig, SignalFailure, StoredTraceSignal},
    worker::Execution,
};
use lens_signals::{RepositoryError, SignalRepository, claimable, config_key};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

use crate::{
    Error,
    state::{Change, ClickHouseState, backoff},
};

const CONFIG: &str = "signals/config";
const KEY_ENCODING: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

#[derive(Clone)]
pub struct Signals(pub ClickHouseState);

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
struct Cooldown {
    failures: u32,
    retry_at: Option<DateTime<Utc>>,
}

impl Cooldown {
    fn active(&self, now: DateTime<Utc>) -> bool {
        self.retry_at.is_some_and(|until| until > now)
    }

    fn after(&self, attempt: &SignalAttempt, now: DateTime<Utc>) -> Self {
        if attempt.status == SignalAttemptStatus::Classified {
            return if self.active(now) {
                self.clone()
            } else {
                Self::default()
            };
        }
        let Some(failure) = &attempt.failure else {
            return self.clone();
        };
        let failures = if self.active(now) {
            self.failures
        } else {
            self.failures.saturating_add(1)
        };
        let seconds = 60_u64
            .saturating_mul(1_u64 << failures.saturating_sub(1).min(5))
            .min(1800);
        let requested = match failure {
            SignalFailure::Transient {
                retry_after_seconds,
            } => retry_after_seconds.unwrap_or(0),
            SignalFailure::Permanent => 0,
        };
        if self.active(now) && requested == 0 {
            return self.clone();
        }
        let delay = if self.active(now) {
            requested
        } else {
            seconds.max(requested)
        };
        let retry_at = now
            .checked_add_signed(TimeDelta::seconds(delay.min(i64::MAX as u64 / 1000) as i64))
            .unwrap_or(DateTime::<Utc>::MAX_UTC);
        Self {
            failures,
            retry_at: Some(
                self.retry_at
                    .map_or(retry_at, |previous| previous.max(retry_at)),
            ),
        }
    }
}

fn cooldown_key(config: &SignalConfig) -> String {
    format!("signals/cooldown/{}", config_key(config))
}

fn cooldown(value: Value) -> Result<Cooldown, RepositoryError> {
    if value.is_null() {
        Ok(Cooldown::default())
    } else {
        decode(value)
    }
}

fn failure(error: impl std::error::Error + Send + Sync + 'static) -> RepositoryError {
    RepositoryError::Unavailable(Box::new(error))
}

fn key(trace_id: &str, trace_ref: &str) -> Result<String, RepositoryError> {
    let identity = serde_json::to_string(&(trace_id, trace_ref)).map_err(failure)?;
    Ok(format!(
        "trace-signal/{}",
        utf8_percent_encode(&identity, KEY_ENCODING)
    ))
}

fn document(value: impl Serialize) -> Result<Value, RepositoryError> {
    serde_json::to_value(value).map_err(failure)
}

fn decode<T: DeserializeOwned>(value: Value) -> Result<T, RepositoryError> {
    let fields: serde_json::Map<String, Value> = serde_json::from_value(value).map_err(failure)?;
    serde_json::from_value(Value::Object(fields)).map_err(failure)
}

impl SignalRepository for Signals {
    async fn get_config(&self) -> Result<SignalConfig, RepositoryError> {
        let record = self.0.read(CONFIG).await.map_err(failure)?;
        if record.value.is_null() {
            return Ok(SignalConfig::default());
        }
        decode(record.value)
    }

    async fn save_config(&self, config: &SignalConfig) -> Result<(), RepositoryError> {
        let value = document(config)?;
        self.0
            .update(CONFIG, |_| value.clone(), 40)
            .await
            .map_err(failure)?;
        Ok(())
    }

    async fn traces(
        &self,
        identities: &[TraceIdentity],
    ) -> Result<Vec<StoredTraceSignal>, RepositoryError> {
        let mut seen = BTreeSet::new();
        let keys = identities
            .iter()
            .map(|identity| key(&identity.trace_id, &identity.trace_ref))
            .collect::<Result<Vec<_>, _>>()?;
        let references = keys
            .iter()
            .filter(|key| seen.insert(key.as_str()))
            .map(String::as_str)
            .collect::<Vec<_>>();
        self.0
            .read_many(&references)
            .await
            .map_err(failure)?
            .into_iter()
            .filter(|snapshot| !snapshot.value.is_null())
            .map(|snapshot| decode(snapshot.value))
            .collect()
    }

    async fn claim(
        &self,
        execution: &Execution,
        config: &SignalConfig,
        claimed_until: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<bool, RepositoryError> {
        let key = key(&execution.trace_id, &execution.trace_ref)?;
        let config_key = config_key(config);
        let pending = document(StoredTraceSignal {
            trace_id: execution.trace_id.clone(),
            trace_ref: execution.trace_ref.clone(),
            config_key: config_key.clone(),
            span_count: execution.span_count,
            claimed_until: Some(claimed_until),
            classified_at: None,
            data: json!({"status": "pending", "scores": {}, "model": config.model, "error": ""}),
        })?;
        for attempt in 0..40 {
            let pause = self.0.read(&cooldown_key(config)).await.map_err(failure)?;
            if cooldown(pause.value)?.active(now) {
                return Ok(false);
            }
            let previous = self.0.read(&key).await.map_err(failure)?;
            let existing = (!previous.value.is_null())
                .then(|| decode(previous.value.clone()))
                .transpose()?;
            if !claimable(execution, existing.as_ref(), &config_key, now) {
                return Ok(false);
            }
            match self
                .0
                .commit(vec![Change {
                    previous,
                    value: pending.clone(),
                }])
                .await
            {
                Ok(()) => return Ok(true),
                Err(Error::StateConflict) => (),
                Err(error) => return Err(failure(error)),
            }
            backoff(attempt).await;
        }
        Ok(false)
    }

    async fn store(
        &self,
        execution: &Execution,
        config: &SignalConfig,
        claimed_until: DateTime<Utc>,
        classified_at: DateTime<Utc>,
        result: &SignalAttempt,
    ) -> Result<(), RepositoryError> {
        let key = key(&execution.trace_id, &execution.trace_ref)?;
        let config_key = config_key(config);
        for attempt in 0..40 {
            let previous = self.0.read(&key).await.map_err(failure)?;
            if previous.value.is_null() {
                return Ok(());
            }
            let existing: StoredTraceSignal = decode(previous.value.clone())?;
            if existing.config_key != config_key || existing.claimed_until != Some(claimed_until) {
                return Ok(());
            }
            let value = document(StoredTraceSignal {
                claimed_until: None,
                classified_at: Some(classified_at),
                data: document(result)?,
                ..existing
            })?;
            let pause = self.0.read(&cooldown_key(config)).await.map_err(failure)?;
            let before = cooldown(pause.value.clone())?;
            let after = before.after(result, classified_at);
            let mut changes = vec![Change { previous, value }];
            if after != before {
                changes.push(Change {
                    previous: pause,
                    value: document(after)?,
                });
            }
            match self.0.commit(changes).await {
                Ok(()) => return Ok(()),
                Err(Error::StateConflict) => (),
                Err(error) => return Err(failure(error)),
            }
            backoff(attempt).await;
        }
        Err(failure(Error::StateConflict))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::first(0, 60)]
    #[case::second(1, 120)]
    #[case::fifth(4, 960)]
    #[case::capped(5, 1800)]
    #[case::saturating(u32::MAX, 1800)]
    fn consecutive_outages_back_off_with_a_bounded_delay(
        #[case] failures: u32,
        #[case] seconds: i64,
    ) {
        let now = DateTime::from_timestamp(0, 0).unwrap();
        let previous = Cooldown {
            failures,
            retry_at: Some(now),
        };
        let attempt: SignalAttempt = serde_json::from_value(json!({"status":"failed","model":"model","failure":{"kind":"transient","retry_after_seconds":null}})).unwrap();
        assert_eq!(
            previous.after(&attempt, now).retry_at,
            Some(now + TimeDelta::seconds(seconds))
        );
    }

    #[rstest]
    fn a_later_retry_after_extends_the_existing_window_without_counting_another_outage() {
        let now = DateTime::from_timestamp(0, 0).unwrap();
        let previous = Cooldown {
            failures: 1,
            retry_at: Some(now + TimeDelta::seconds(60)),
        };
        let attempt: SignalAttempt = serde_json::from_value(json!({"status":"failed","model":"model","failure":{"kind":"transient","retry_after_seconds":600}})).unwrap();
        let updated = previous.after(&attempt, now + TimeDelta::seconds(1));
        assert_eq!(updated.failures, 1);
        assert_eq!(updated.retry_at, Some(now + TimeDelta::seconds(601)));
    }

    #[rstest]
    fn successful_recovery_resets_exponential_backoff() {
        let now = DateTime::from_timestamp(0, 0).unwrap();
        let previous = Cooldown {
            failures: 5,
            retry_at: Some(now),
        };
        let attempt: SignalAttempt =
            serde_json::from_value(json!({"status":"classified","model":"model"})).unwrap();
        assert_eq!(previous.after(&attempt, now), Cooldown::default());
    }
}
