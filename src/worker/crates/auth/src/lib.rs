#![forbid(unsafe_code)]

mod error;
mod gateway;
pub mod ingestion;

use chrono::{DateTime, Utc};
use lens_contract::auth::{Identity, Role, SessionView};
use sha2::{Digest, Sha256};
use std::future::Future;
use subtle::ConstantTimeEq;

pub use error::{Error, IngestionError, StoreError};

pub const SESSION_LIFETIME_SECONDS: i64 = 8 * 60 * 60;
pub const GATEWAY_SESSION_LIFETIME_SECONDS: i64 = 60 * 60;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Session {
    pub expires_at: DateTime<Utc>,
    pub identity: Option<Identity>,
}

#[derive(Debug)]
pub struct SignIn {
    pub view: SessionView,
    pub lifetime_seconds: i64,
}

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
        session: &Session,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn find(
        &self,
        id: &SessionId,
    ) -> impl Future<Output = Result<Option<Session>, StoreError>> + Send;
    fn revoke(&self, id: &SessionId) -> impl Future<Output = Result<(), StoreError>> + Send;
    fn spend_login(
        &self,
        login: &SessionId,
        expires_at: DateTime<Utc>,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
}

pub struct Settings {
    admin_hash: [u8; 32],
    slack_investigator: Option<([u8; 32], String)>,
    gateway_secret: Option<String>,
    gateway_login_url: Option<String>,
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
        if gateway_secret
            .as_deref()
            .is_some_and(|secret| secret.chars().count() < 32)
        {
            return Err(Error::Configuration(
                "gateway secret must contain at least 32 characters",
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
            slack_investigator: None,
            gateway_secret,
            gateway_login_url: None,
            origin: format!("{}://{}", url.scheme(), authority),
            secure_cookie: public_url.starts_with("https://"),
        })
    }

    pub fn with_slack_investigator(mut self, key: &str, team_id: &str) -> Result<Self, Error> {
        if !(32..=512).contains(&key.len())
            || self.is_admin_token(key)
            || team_id.is_empty()
            || team_id.len() > 256
        {
            return Err(Error::Configuration(
                "invalid Slack investigator key or team",
            ));
        }
        self.slack_investigator = Some((Sha256::digest(key.as_bytes()).into(), team_id.into()));
        Ok(self)
    }

    pub fn with_gateway_login(mut self, gateway_url: &str) -> Result<Self, Error> {
        let url =
            url::Url::parse(gateway_url).map_err(|_| Error::Configuration("gateway public URL"))?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(Error::Configuration("gateway public URL"));
        }
        let base = url.as_str().trim_end_matches('/');
        let base = base.strip_suffix("/v1").unwrap_or(base);
        self.gateway_login_url = self
            .gateway_secret
            .is_some()
            .then(|| format!("{base}/lens/login"));
        Ok(self)
    }

    pub fn gateway_login_url(&self) -> Option<&str> {
        self.gateway_login_url.as_deref()
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
    ) -> Result<SignIn, Error> {
        let (identity, lifetime_seconds) = if self.settings.is_admin_token(token) {
            (None, SESSION_LIFETIME_SECONDS)
        } else {
            let secret = self
                .settings
                .gateway_secret
                .as_deref()
                .ok_or(Error::Unauthorized("Invalid Lens setup token"))?;
            let identity = gateway::login_identity(token, secret, now)
                .map_err(|_| Error::Unauthorized("Invalid Lens setup token"))?;
            self.sessions
                .spend_login(
                    &SessionId::for_token(token),
                    now + chrono::Duration::seconds(gateway::LOGIN_LIFETIME_SECONDS),
                )
                .await
                .map_err(|error| match error {
                    StoreError::Conflict => Error::Unauthorized("Invalid Lens setup token"),
                    error => Error::from(error),
                })?;
            (Some(identity), GATEWAY_SESSION_LIFETIME_SECONDS)
        };
        self.sessions
            .create(
                &SessionId::for_token(session),
                &Session {
                    expires_at: now + chrono::Duration::seconds(lifetime_seconds),
                    identity: identity.clone(),
                },
            )
            .await?;
        Ok(SignIn {
            view: session_view(identity.unwrap_or_else(local_admin)),
            lifetime_seconds,
        })
    }

    pub async fn authenticate(
        &self,
        credentials: Credentials<'_>,
        now: DateTime<Utc>,
    ) -> Result<Identity, Error> {
        self.authenticate_request(credentials, now, false).await
    }

    pub async fn authenticate_slack(
        &self,
        credentials: Credentials<'_>,
        now: DateTime<Utc>,
    ) -> Result<Identity, Error> {
        self.authenticate_request(credentials, now, true).await
    }

    async fn authenticate_request(
        &self,
        credentials: Credentials<'_>,
        now: DateTime<Utc>,
        allow_slack: bool,
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
            if let Some((hash, team_id)) = &self.settings.slack_investigator
                && bool::from(hash.ct_eq(&Sha256::digest(token.as_bytes())))
            {
                if !allow_slack {
                    return Err(Error::Unauthorized("Invalid Lens credential"));
                }
                return Ok(Identity {
                    user_role: Role::SlackInvestigator,
                    team_id: Some(team_id.clone()),
                    ..Identity::default()
                });
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
        let stored = self
            .sessions
            .find(&SessionId::for_token(session))
            .await?
            .filter(|stored| stored.expires_at > now)
            .ok_or(Error::Unauthorized("Lens session has expired"))?;
        Ok(stored.identity.unwrap_or_else(local_admin))
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
    if identity.user_role == Role::SlackInvestigator {
        let team_id = identity.team_id.as_ref().filter(|id| !id.is_empty())?;
        return Some(ReadScope::OwnedRows {
            user_id: String::new(),
            team_ids: vec![team_id.clone()],
        });
    }
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
