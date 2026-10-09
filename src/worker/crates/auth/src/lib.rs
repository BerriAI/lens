#![forbid(unsafe_code)]

mod error;
mod gateway;

use chrono::{DateTime, Utc};
use lens_contract::auth::{Identity, Role, SessionView};
use sha2::{Digest, Sha256};
use std::future::Future;
use subtle::ConstantTimeEq;

pub use error::{Error, StoreError};

pub const SESSION_LIFETIME_SECONDS: i64 = 8 * 60 * 60;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionId(String);

impl SessionId {
    pub fn for_token(token: &str) -> Self {
        Self(format!("{:x}", Sha256::digest(token.as_bytes())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub trait SessionRepository: Send + Sync {
    fn create(
        &self,
        id: &SessionId,
        expires_at: DateTime<Utc>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn expires_at(
        &self,
        id: &SessionId,
    ) -> impl Future<Output = Result<Option<DateTime<Utc>>, StoreError>> + Send;
    fn revoke(&self, id: &SessionId) -> impl Future<Output = Result<(), StoreError>> + Send;
}

pub struct Settings {
    admin_hash: [u8; 32],
    gateway_secret: Option<String>,
    origin: String,
    secure_cookie: bool,
}

impl Settings {
    pub fn new(
        admin_token: &str,
        gateway_secret: Option<String>,
        public_url: &str,
    ) -> Result<Self, Error> {
        if admin_token.chars().count() < 32 {
            return Err(Error::Configuration(
                "setup token must contain at least 32 characters",
            ));
        }
        let url = url::Url::parse(public_url).map_err(|_| Error::Configuration("public URL"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || !url.path().trim_matches('/').is_empty()
            || url.query().is_some_and(|value| !value.is_empty())
            || url.fragment().is_some_and(|value| !value.is_empty())
        {
            return Err(Error::Configuration("public URL"));
        }
        let authority = public_url
            .split_once("://")
            .ok_or(Error::Configuration("public URL"))?
            .1
            .split(['/', '?', '#'])
            .next()
            .ok_or(Error::Configuration("public URL"))?;
        Ok(Self {
            admin_hash: Sha256::digest(admin_token.as_bytes()).into(),
            gateway_secret,
            origin: format!("{}://{}", url.scheme(), authority),
            secure_cookie: public_url.starts_with("https://"),
        })
    }

    pub fn secure_cookie(&self) -> bool {
        self.secure_cookie
    }

    fn is_admin_token(&self, token: &str) -> bool {
        self.admin_hash
            .ct_eq(&Sha256::digest(token.as_bytes()))
            .into()
    }
}

pub struct Credentials<'a> {
    pub authorization: Option<&'a str>,
    pub session: Option<&'a str>,
    pub method: &'a str,
    pub origin: Option<&'a str>,
}

pub struct Authentication<R> {
    pub settings: Settings,
    pub sessions: R,
}

impl<R: SessionRepository> Authentication<R> {
    pub async fn sign_in(
        &self,
        token: &str,
        session: &str,
        now: DateTime<Utc>,
    ) -> Result<SessionView, Error> {
        if !self.settings.is_admin_token(token) {
            return Err(Error::Unauthorized("Invalid Lens setup token"));
        }
        self.sessions
            .create(
                &SessionId::for_token(session),
                now + chrono::Duration::seconds(SESSION_LIFETIME_SECONDS),
            )
            .await?;
        Ok(session_view(local_admin()))
    }

    pub async fn authenticate(
        &self,
        credentials: Credentials<'_>,
        now: DateTime<Utc>,
    ) -> Result<Identity, Error> {
        if let Some(header) = credentials
            .authorization
            .filter(|header| !header.is_empty())
        {
            let (scheme, token) = header
                .split_once(' ')
                .ok_or(Error::Unauthorized("Use a Lens bearer credential"))?;
            if !scheme.eq_ignore_ascii_case("bearer") || token.is_empty() {
                return Err(Error::Unauthorized("Use a Lens bearer credential"));
            }
            if self.settings.is_admin_token(token) {
                return Ok(local_admin());
            }
            let secret = self
                .settings
                .gateway_secret
                .as_deref()
                .ok_or(Error::Unauthorized("Invalid Lens credential"))?;
            return gateway::identity(token, secret, now);
        }
        let session = credentials
            .session
            .ok_or(Error::Unauthorized("Sign in to Lens"))?;
        if !matches!(credentials.method, "GET" | "HEAD" | "OPTIONS")
            && credentials.origin != Some(self.settings.origin.as_str())
        {
            return Err(Error::OriginMismatch);
        }
        if !self
            .sessions
            .expires_at(&SessionId::for_token(session))
            .await?
            .is_some_and(|expires| expires > now)
        {
            return Err(Error::Unauthorized("Lens session has expired"));
        }
        Ok(local_admin())
    }

    pub async fn sign_out(
        &self,
        credentials: Credentials<'_>,
        now: DateTime<Utc>,
    ) -> Result<(), Error> {
        let session = credentials.session;
        self.authenticate(credentials, now).await?;
        if let Some(token) = session {
            self.sessions.revoke(&SessionId::for_token(token)).await?;
        }
        Ok(())
    }
}

pub fn local_admin() -> Identity {
    Identity {
        user_id: Some("lens-admin".into()),
        user_role: Role::ProxyAdmin,
        ..Identity::default()
    }
}

pub fn session_view(identity: Identity) -> SessionView {
    SessionView {
        user_id: identity.user_id.unwrap_or_default(),
        user_role: identity.user_role,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadScope {
    AllRows,
    OwnedRows {
        user_id: String,
        team_ids: Vec<String>,
    },
}

pub fn trace_read_scope(identity: &Identity) -> Option<ReadScope> {
    if matches!(
        identity.user_role,
        Role::ProxyAdmin | Role::ProxyAdminViewer
    ) {
        return Some(ReadScope::AllRows);
    }
    let user_id = identity.user_id.as_ref().filter(|id| !id.is_empty())?;
    Some(ReadScope::OwnedRows {
        user_id: user_id.clone(),
        team_ids: identity.log_team_ids.clone(),
    })
}
