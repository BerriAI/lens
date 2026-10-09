use axum::{
    extract::FromRequestParts,
    http::{
        HeaderMap, Method,
        header::{self, AUTHORIZATION},
        request::Parts,
    },
};

use crate::ApiError;

pub(crate) async fn identity<R: lens_auth::SessionRepository>(
    auth: &lens_auth::Authentication<R>,
    headers: &HeaderMap,
    method: &Method,
) -> Result<lens_contract::auth::Identity, lens_auth::Error> {
    let session = session_cookie(headers);
    auth.authenticate(
        credentials(headers, method, session.as_deref())?,
        chrono::Utc::now(),
    )
    .await
}

pub(crate) fn credentials<'a>(
    headers: &'a HeaderMap,
    method: &'a Method,
    session: Option<&'a str>,
) -> Result<lens_auth::Credentials<'a>, lens_auth::Error> {
    let authorization = headers
        .get(header::AUTHORIZATION)
        .map(|header| header.to_str())
        .transpose()
        .map_err(|_| lens_auth::Error::Unauthorized("Use a Lens bearer credential"))?;
    Ok(lens_auth::Credentials {
        authorization,
        session,
        method: method.as_str(),
        origin: headers
            .get(header::ORIGIN)
            .and_then(|value| value.to_str().ok()),
    })
}

pub(crate) fn session_cookie(headers: &HeaderMap) -> Option<String> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    cookie::Cookie::split_parse(cookies)
        .filter_map(Result::ok)
        .filter(|cookie| cookie.name() == "lens_session")
        .map(|cookie| cookie.value_trimmed().to_owned())
        .last()
}

pub struct BearerToken(String);

impl BearerToken {
    pub fn token(&self) -> &str {
        &self.0
    }
}

impl<S> FromRequestParts<S> for BearerToken
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let Some(header) = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
        else {
            return Err(ApiError::Unauthorized);
        };

        let Some((scheme, token)) = header.split_once(' ') else {
            return Err(ApiError::Unauthorized);
        };
        if !scheme.eq_ignore_ascii_case("Bearer")
            || token.is_empty()
            || token.bytes().any(|byte| byte.is_ascii_whitespace())
        {
            return Err(ApiError::Unauthorized);
        }

        Ok(Self(token.to_owned()))
    }
}
