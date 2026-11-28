use std::future::Future;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use lens_contract::{
    auth::{Identity, Role},
    mcp::{
        McpActivity, McpAuthentication, McpCall, McpConnection, McpConnectionCreated,
        McpCreateRequest,
    },
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::{McpError, ReadScope, StoreError, trace_read_scope};

pub const CONNECTION_LIMIT: usize = 1000;
pub const RETENTION_DAYS: u16 = 30;

#[derive(Clone, Deserialize, Serialize)]
pub struct StoredConnection {
    pub connection: McpConnection,
    pub token_hash: Option<[u8; 32]>,
}

pub struct ActivityQuery {
    pub owner_id: Option<String>,
    pub connection_id: Option<String>,
    pub from_ms: i64,
    pub to_ms: i64,
    pub before: Option<(i64, String)>,
}

pub trait McpRepository: Send + Sync {
    fn mcp_connections(
        &self,
    ) -> impl Future<Output = Result<Vec<StoredConnection>, StoreError>> + Send;
    fn mcp_insert(
        &self,
        connection: &StoredConnection,
    ) -> impl Future<Output = Result<(), McpError>> + Send;
    fn mcp_revoke(
        &self,
        id: &str,
        at: DateTime<Utc>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn mcp_record(&self, call: &McpCall) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn mcp_activity(
        &self,
        query: ActivityQuery,
    ) -> impl Future<Output = Result<McpActivity, StoreError>> + Send;
}

pub fn authentication(identity: &Identity) -> McpAuthentication {
    if identity.token.is_some() {
        McpAuthentication::Gateway
    } else {
        McpAuthentication::Token
    }
}

pub fn can_manage_all(identity: &Identity) -> bool {
    matches!(
        identity.user_role,
        Role::ProxyAdmin | Role::ProxyAdminViewer
    )
}

pub fn can_create(identity: &Identity) -> bool {
    trace_read_scope(identity).is_some()
        && !matches!(
            identity.user_role,
            Role::ProxyAdminViewer | Role::InternalUserViewer | Role::SlackInvestigator
        )
        && (authentication(identity) == McpAuthentication::Gateway
            || identity.user_role == Role::ProxyAdmin)
}

pub fn scope_label(identity: &Identity) -> &'static str {
    match trace_read_scope(identity) {
        Some(ReadScope::AllRows) => "All workspace traces",
        Some(ReadScope::OwnedRows { .. }) => "Your traces and permitted teams",
        None => "No trace access",
    }
}

pub fn authorize_manager(identity: &Identity, record: &McpConnection) -> Result<(), McpError> {
    if can_create(identity)
        && (identity.user_role == Role::ProxyAdmin
            || identity.user_id.as_deref() == Some(&record.owner_id))
    {
        Ok(())
    } else {
        Err(McpError::Forbidden)
    }
}

pub async fn create<R: McpRepository>(
    repository: &R,
    identity: &Identity,
    request: McpCreateRequest,
    now: DateTime<Utc>,
) -> Result<McpConnectionCreated, McpError> {
    if !can_create(identity) {
        return Err(McpError::Forbidden);
    }
    let id = uuid::Uuid::parse_str(&request.request_id)
        .map_err(|_| McpError::Invalid("Invalid connection request ID"))?
        .to_string();
    if request.name.trim().is_empty()
        || request.name.chars().count() > 128
        || request.client.trim().is_empty()
        || request.client.chars().count() > 64
    {
        return Err(McpError::Invalid(
            "Enter a name up to 128 characters and a client up to 64 characters",
        ));
    }
    if !(1..=365).contains(&request.expires_in_days) {
        return Err(McpError::Invalid("Expiry must be between 1 and 365 days"));
    }
    let kind = authentication(identity);
    if request
        .owner_label
        .as_ref()
        .is_some_and(|label| label.trim().is_empty() || label.chars().count() > 128)
    {
        return Err(McpError::Invalid(
            "Owner label must contain 1 to 128 characters",
        ));
    }
    if kind == McpAuthentication::Gateway && request.owner_label.is_some() {
        return Err(McpError::Invalid(
            "Gateway owners come from the signed-in identity",
        ));
    }
    let token = if kind == McpAuthentication::Token {
        let mut bytes = [0_u8; 32];
        rand::rngs::OsRng
            .try_fill_bytes(&mut bytes)
            .map_err(McpError::Random)?;
        Some(format!("lens-mcp-{id}.{}", URL_SAFE_NO_PAD.encode(bytes)))
    } else {
        None
    };
    let connection = McpConnection {
        id,
        name: request.name.trim().into(),
        client: request.client.trim().into(),
        owner_id: identity
            .user_id
            .clone()
            .filter(|id| !id.is_empty())
            .ok_or(McpError::Forbidden)?,
        owner_name: if kind == McpAuthentication::Token {
            request
                .owner_label
                .unwrap_or_else(|| "Deployment administrator".into())
        } else {
            identity
                .user_name
                .clone()
                .or_else(|| identity.user_id.clone())
                .unwrap_or_default()
        },
        authentication: kind,
        created_at: now,
        expires_at: now + chrono::Duration::days(i64::from(request.expires_in_days)),
        revoked_at: None,
    };
    repository
        .mcp_insert(&StoredConnection {
            connection: connection.clone(),
            token_hash: token
                .as_ref()
                .map(|token| Sha256::digest(token.as_bytes()).into()),
        })
        .await?;
    Ok(McpConnectionCreated { connection, token })
}

pub fn token_id(token: &str) -> Option<String> {
    let id = token.strip_prefix("lens-mcp-")?.split_once('.')?.0;
    uuid::Uuid::parse_str(id).ok().map(|id| id.to_string())
}

pub fn check_active(record: &McpConnection, now: DateTime<Utc>) -> Result<(), McpError> {
    if record.revoked_at.is_some() {
        return Err(McpError::Revoked);
    }
    if record.expires_at <= now {
        return Err(McpError::Expired);
    }
    Ok(())
}

pub fn check_token(record: &StoredConnection, token: &str) -> Result<(), McpError> {
    let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
    if record.connection.authentication != McpAuthentication::Token
        || !record
            .token_hash
            .is_some_and(|hash| bool::from(hash.ct_eq(&digest)))
    {
        return Err(McpError::Unauthorized);
    }
    Ok(())
}

pub fn legacy_id(identity: &Identity) -> String {
    let source = identity.token.as_deref().unwrap_or("standalone-admin");
    format!(
        "legacy-{:x}",
        Sha256::digest(
            format!(
                "{}:{source}",
                identity.user_id.as_deref().unwrap_or_default()
            )
            .as_bytes()
        )
    )
}
