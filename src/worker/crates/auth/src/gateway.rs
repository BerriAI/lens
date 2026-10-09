use chrono::{DateTime, Utc};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use lens_contract::auth::Identity;
use serde::{Deserialize, Deserializer};

use crate::Error;

const CLOCK_SKEW_SECONDS: i64 = 5;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Claims {
    iss: String,
    aud: String,
    sub: String,
    #[serde(deserialize_with = "numeric_date")]
    iat: i64,
    #[serde(deserialize_with = "numeric_date")]
    exp: i64,
    identity: Identity,
}

pub(super) fn identity(token: &str, secret: &str, now: DateTime<Utc>) -> Result<Identity, Error> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_issuer(&["litellm"]);
    validation.set_audience(&["litellm-lens"]);
    validation.set_required_spec_claims(&["iss", "aud", "sub"]);
    validation.validate_exp = false;
    validation.validate_nbf = false;
    let claims = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map_err(|_| Error::Unauthorized("Invalid or expired gateway identity"))?
    .claims;
    if claims.iss != "litellm"
        || claims.aud != "litellm-lens"
        || claims.iat > now.timestamp().saturating_add(CLOCK_SKEW_SECONDS)
        || claims.exp <= now.timestamp()
    {
        return Err(Error::Unauthorized("Invalid or expired gateway identity"));
    }
    let subject = claims
        .identity
        .user_id
        .as_deref()
        .filter(|user| !user.is_empty())
        .or(claims.identity.token.as_deref());
    if claims
        .exp
        .checked_sub(claims.iat)
        .is_none_or(|lifetime| !(1..=60).contains(&lifetime))
        || subject != Some(claims.sub.as_str())
    {
        return Err(Error::Unauthorized("Invalid gateway identity scope"));
    }
    Ok(claims.identity)
}

fn numeric_date<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    let integer = match value {
        serde_json::Value::Number(number) => number.as_i64().or_else(|| {
            let number = number.as_f64()?;
            (number.fract() == 0.0 && number >= i64::MIN as f64 && number < -(i64::MIN as f64))
                .then_some(number as i64)
        }),
        serde_json::Value::String(text) => text.trim().parse().ok(),
        serde_json::Value::Bool(value) => Some(i64::from(value)),
        _ => None,
    };
    integer.ok_or_else(|| serde::de::Error::custom("invalid numeric date"))
}
