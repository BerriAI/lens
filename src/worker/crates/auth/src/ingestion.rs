use std::future::Future;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use lens_contract::{
    auth::{Identity, Role},
    ingestion::{
        IngestionCredential, IngestionKey, IngestionKeyCreated, IngestionKeyRequest,
        IngestionSnapshot, IngestionTenant,
    },
};
use rand::RngCore;
use sha2::{Digest, Sha256};

use crate::IngestionError;

pub const INGESTION_KEY_LIMIT: usize = 10_000;

pub trait IngestionRepository: Send + Sync {
    fn list(&self) -> impl Future<Output = Result<Vec<IngestionKey>, IngestionError>> + Send;
    fn insert(&self, key: &IngestionKey)
    -> impl Future<Output = Result<(), IngestionError>> + Send;
    fn revoke(&self, id: &str) -> impl Future<Output = Result<(), IngestionError>> + Send;
}

#[derive(Clone)]
pub struct Ingestion<R>(pub R);

impl<R: IngestionRepository> Ingestion<R> {
    pub async fn create(
        &self,
        identity: &Identity,
        request: IngestionKeyRequest,
        now: DateTime<Utc>,
    ) -> Result<IngestionKeyCreated, IngestionError> {
        authorize(identity, true)?;
        validate_length("name", &request.name, 1, 128)?;
        validate_length("team_id", &request.team_id, 0, 256)?;
        if request.expires_at.is_some_and(|expires| expires <= now) {
            return Err(IngestionError::InvalidExpiry);
        }
        let mut bytes = [0_u8; 40];
        rand::rngs::OsRng
            .try_fill_bytes(&mut bytes)
            .map_err(IngestionError::Random)?;
        let key = format!(
            "lens-trace-{}-{}",
            now.timestamp(),
            URL_SAFE_NO_PAD.encode(bytes)
        );
        let record = IngestionKey {
            id: uuid::Uuid::new_v4().to_string(),
            name: request.name,
            tenant: IngestionTenant {
                team_id: request.team_id,
                user_id: identity.user_id.clone().unwrap_or_default(),
                org_id: String::new(),
                api_key_hash: format!("{:x}", Sha256::digest(key.as_bytes())),
            },
            created_at: now,
            expires_at: request.expires_at.map(|expires| expires.timestamp()),
        };
        self.0.insert(&record).await?;
        Ok(IngestionKeyCreated {
            key,
            record,
            active: false,
        })
    }

    pub async fn list(&self, identity: &Identity) -> Result<Vec<IngestionKey>, IngestionError> {
        authorize(identity, false)?;
        self.0.list().await
    }

    pub async fn revoke(&self, identity: &Identity, id: &str) -> Result<(), IngestionError> {
        authorize(identity, true)?;
        self.0.revoke(id).await
    }

    pub async fn current_snapshot(
        &self,
        now: DateTime<Utc>,
    ) -> Result<IngestionSnapshot, IngestionError> {
        let issued_at = now.timestamp();
        let keys = self
            .0
            .list()
            .await?
            .into_iter()
            .filter(|key| key.expires_at.is_none_or(|expiry| expiry > issued_at))
            .map(|key| IngestionCredential {
                token_hash: key.tenant.api_key_hash.clone(),
                tenant: key.tenant,
                expires_at: key.expires_at,
            })
            .collect();
        Ok(IngestionSnapshot { issued_at, keys })
    }
}

fn authorize(identity: &Identity, write: bool) -> Result<(), IngestionError> {
    if write && identity.user_role != Role::ProxyAdmin {
        return Err(IngestionError::Forbidden(
            "Only proxy admins can configure or run Lens",
        ));
    }
    if !matches!(
        identity.user_role,
        Role::ProxyAdmin | Role::ProxyAdminViewer
    ) {
        return Err(IngestionError::Forbidden(
            "Lens requires proxy administrator access",
        ));
    }
    Ok(())
}

fn validate_length(
    field: &'static str,
    value: &str,
    min: usize,
    max: usize,
) -> Result<(), IngestionError> {
    if !(min..=max).contains(&value.chars().count()) {
        return Err(IngestionError::InvalidLength { field, min, max });
    }
    Ok(())
}
