use chrono::{DateTime, Utc};
use lens_auth::{Authentication, Credentials, SessionId, SessionRepository, Settings, StoreError};
use rstest::fixture;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

pub const ADMIN: &str = "parity-admin-token-32-characters-long";
pub const SECRET: &str = "test-gateway-secret-32-characters-long";
pub const ORIGIN: &str = "https://lens.test";

#[derive(Clone, Default)]
pub struct Memory {
    pub values: Arc<Mutex<BTreeMap<String, DateTime<Utc>>>>,
    pub unavailable: bool,
    pub identities: Arc<Mutex<BTreeMap<String, lens_contract::auth::Identity>>>,
}

impl Memory {
    fn ready(&self) -> Result<(), StoreError> {
        if self.unavailable {
            Err(StoreError::Unavailable(Box::new(std::io::Error::other(
                "private",
            ))))
        } else {
            Ok(())
        }
    }
}

impl SessionRepository for Memory {
    async fn create_identity(
        &self,
        id: &SessionId,
        expires: DateTime<Utc>,
        identity: &lens_contract::auth::Identity,
    ) -> Result<(), StoreError> {
        self.create(id, expires).await?;
        self.identities
            .lock()
            .unwrap()
            .insert(id.as_str().into(), identity.clone());
        Ok(())
    }
    async fn identity(
        &self,
        id: &SessionId,
    ) -> Result<Option<lens_contract::auth::Identity>, StoreError> {
        self.ready()?;
        Ok(self
            .values
            .lock()
            .unwrap()
            .contains_key(id.as_str())
            .then(|| {
                self.identities
                    .lock()
                    .unwrap()
                    .get(id.as_str())
                    .cloned()
                    .unwrap_or_else(lens_auth::local_admin)
            }))
    }
    async fn create(&self, id: &SessionId, expires: DateTime<Utc>) -> Result<(), StoreError> {
        self.ready()?;
        let mut values = self.values.lock().unwrap();
        if values.contains_key(id.as_str()) {
            return Err(StoreError::Conflict);
        }
        values.insert(id.as_str().to_owned(), expires);
        Ok(())
    }
    async fn expires_at(&self, id: &SessionId) -> Result<Option<DateTime<Utc>>, StoreError> {
        self.ready()?;
        Ok(self.values.lock().unwrap().get(id.as_str()).copied())
    }
    async fn revoke(&self, id: &SessionId) -> Result<(), StoreError> {
        self.ready()?;
        self.values.lock().unwrap().remove(id.as_str());
        self.identities.lock().unwrap().remove(id.as_str());
        Ok(())
    }
}

#[fixture]
pub fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

#[fixture]
pub fn authentication() -> Authentication<Memory> {
    Authentication {
        settings: Settings::new(ADMIN, Some(SECRET.to_owned()), ORIGIN).unwrap(),
        sessions: Memory::default(),
    }
}

pub fn cookie(session: &str) -> Credentials<'_> {
    Credentials {
        authorization: None,
        session: Some(session),
        method: "GET",
        origin: None,
    }
}

pub fn bearer(authorization: &str) -> Credentials<'_> {
    Credentials {
        authorization: Some(authorization),
        session: Some("active"),
        method: "DELETE",
        origin: None,
    }
}
