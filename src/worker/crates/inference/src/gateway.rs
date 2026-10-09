use chrono::{DateTime, Utc};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::Serialize;
use url::Url;

use crate::Error;

pub const GATEWAY_HEADER: &str = "x-lens-internal";

#[derive(Clone)]
pub struct GatewayIdentity {
    base: Url,
    key: EncodingKey,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayPurpose {
    Analysis,
    Signals,
}

#[derive(Serialize)]
struct Claims {
    iss: &'static str,
    aud: &'static str,
    sub: &'static str,
    purpose: GatewayPurpose,
    iat: i64,
    exp: i64,
}

impl GatewayIdentity {
    pub fn new(base: Url, secret: &str) -> Result<Self, Error> {
        if !matches!(base.scheme(), "http" | "https")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || !(32..=512).contains(&secret.len())
        {
            return Err(Error::GatewayConfiguration);
        }
        Ok(Self {
            base,
            key: EncodingKey::from_secret(secret.as_bytes()),
        })
    }

    pub fn token(
        &self,
        endpoint: &Url,
        purpose: GatewayPurpose,
        now: DateTime<Utc>,
    ) -> Result<Option<String>, Error> {
        let prefix = self.base.path().trim_end_matches('/');
        if endpoint.origin() != self.base.origin()
            || !endpoint.path().starts_with(&format!("{prefix}/"))
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
        {
            return Ok(None);
        }
        let claims = Claims {
            iss: "litellm-lens",
            aud: "litellm",
            sub: "lens-internal",
            purpose,
            iat: now.timestamp(),
            exp: now.timestamp() + 30,
        };
        Ok(Some(encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &self.key,
        )?))
    }
}
