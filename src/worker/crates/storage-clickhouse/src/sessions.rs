use chrono::{DateTime, Utc};
use lens_auth::{SessionId, SessionRepository, StoreError};
use lens_contract::auth::Identity;
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
    #[serde(default = "lens_auth::local_admin")]
    identity: Identity,
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
        self.create_identity(id, expires_at, &lens_auth::local_admin())
            .await
    }

    async fn identity(&self, id: &SessionId) -> Result<Option<Identity>, StoreError> {
        let stored = self.0.read(&key(id)).await.map_err(failure)?;
        if stored.value.is_null() {
            return Ok(None);
        }
        let session: Session = serde_json::from_value(stored.value)
            .map_err(|error| StoreError::Unavailable(Box::new(error)))?;
        Ok(Some(session.identity))
    }

    async fn create_identity(
        &self,
        id: &SessionId,
        expires_at: DateTime<Utc>,
        identity: &Identity,
    ) -> Result<(), StoreError> {
        let previous = self.0.read(&key(id)).await.map_err(failure)?;
        if !previous.value.is_null() {
            return Err(StoreError::Conflict);
        }
        let value = serde_json::to_value(Session {
            expires_at,
            identity: identity.clone(),
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use lens_contract::auth::Role;
    use rstest::rstest;
    use serde_json::json;

    #[rstest]
    fn legacy_sessions_remain_administrators() {
        let session: Session =
            serde_json::from_value(json!({"expires_at":"2030-01-01T00:00:00Z"})).unwrap();
        assert_eq!(session.identity, lens_auth::local_admin());
    }

    #[rstest]
    fn identity_is_persisted_without_privilege_upgrade() {
        let identity = Identity {
            user_id: Some("google:subject".into()),
            user_role: Role::InternalUserViewer,
            ..Identity::default()
        };
        let session = Session {
            expires_at: Utc::now(),
            identity: identity.clone(),
        };
        let restored: Session =
            serde_json::from_value(serde_json::to_value(session).unwrap()).unwrap();
        assert_eq!(restored.identity, identity);
    }

    #[rstest]
    #[case::null(json!(null))]
    #[case::unknown_role(json!({"user_id":"google:subject","user_role":"unknown"}))]
    fn corrupt_identity_fails_closed(#[case] identity: Value) {
        assert!(
            serde_json::from_value::<Session>(
                json!({"expires_at":"2030-01-01T00:00:00Z","identity":identity})
            )
            .is_err()
        );
    }
}
