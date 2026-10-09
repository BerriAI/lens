use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use lens_auth::{
    IngestionError,
    ingestion::{Ingestion, IngestionRepository},
    local_admin,
};
use lens_contract::{auth::Identity, ingestion::IngestionKey};
use rstest::fixture;

#[derive(Clone, Default)]
pub struct Memory(pub Arc<Mutex<Vec<IngestionKey>>>);

impl IngestionRepository for Memory {
    async fn list(&self) -> Result<Vec<IngestionKey>, IngestionError> {
        Ok(self.0.lock().unwrap().clone())
    }

    async fn insert(&self, key: &IngestionKey) -> Result<(), IngestionError> {
        self.0.lock().unwrap().push(key.clone());
        Ok(())
    }

    async fn revoke(&self, id: &str) -> Result<(), IngestionError> {
        self.0.lock().unwrap().retain(|key| key.id != id);
        Ok(())
    }
}

#[fixture]
pub fn ingestion() -> Ingestion<Memory> {
    Ingestion(Memory::default())
}

#[fixture]
pub fn now() -> DateTime<Utc> {
    "2026-03-01T12:00:00.123456Z".parse().unwrap()
}

#[fixture]
pub fn admin() -> Identity {
    Identity {
        team_id: Some("identity-team".into()),
        org_id: Some("identity-org".into()),
        user_id: Some("owner".into()),
        ..local_admin()
    }
}
