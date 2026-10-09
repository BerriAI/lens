use chrono::{DateTime, Utc};
use lens_auth::{SessionId, SessionRepository, StoreError};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    Error,
    state::{Change, ClickHouseState},
};

#[derive(Clone)]
pub struct Sessions(pub ClickHouseState);

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Session {
    expires_at: DateTime<Utc>,
}

fn key(id: &SessionId) -> String {
    format!("session/%5B%22{}%22%5D", id.as_str())
}

fn failure(error: Error) -> StoreError {
    match error {
        Error::StateConflict | Error::StateExists => StoreError::Conflict,
        error => StoreError::Unavailable(Box::new(error)),
    }
}

impl SessionRepository for Sessions {
    async fn create(&self, id: &SessionId, expires_at: DateTime<Utc>) -> Result<(), StoreError> {
        let previous = self.0.read(&key(id)).await.map_err(failure)?;
        if !previous.value.is_null() {
            return Err(StoreError::Conflict);
        }
        let value = serde_json::to_value(Session { expires_at })
            .map_err(|error| StoreError::Unavailable(Box::new(error)))?;
        self.0
            .commit(vec![Change { previous, value }])
            .await
            .map_err(failure)
    }

    async fn expires_at(&self, id: &SessionId) -> Result<Option<DateTime<Utc>>, StoreError> {
        let stored = self.0.read(&key(id)).await.map_err(failure)?;
        if stored.value.is_null() {
            return Ok(None);
        }
        let session: Session = serde_json::from_value(stored.value)
            .map_err(|error| StoreError::Unavailable(Box::new(error)))?;
        Ok(Some(session.expires_at))
    }

    async fn revoke(&self, id: &SessionId) -> Result<(), StoreError> {
        self.0
            .update(&key(id), |_| Value::Null, 40)
            .await
            .map_err(failure)?;
        Ok(())
    }
}
