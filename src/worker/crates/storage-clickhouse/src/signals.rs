use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use lens_contract::{
    feedback::TraceIdentity,
    signals::{SignalAttempt, SignalConfig, StoredTraceSignal},
    worker::Execution,
};
use lens_signals::{RepositoryError, SignalRepository, claimable, config_key};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Serialize, de::DeserializeOwned};
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
            match self.0.commit(vec![Change { previous, value }]).await {
                Ok(()) => return Ok(()),
                Err(Error::StateConflict) => (),
                Err(error) => return Err(failure(error)),
            }
            backoff(attempt).await;
        }
        Err(failure(Error::StateConflict))
    }
}
