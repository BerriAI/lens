use std::sync::Arc;

use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use lens_auth::{Authentication, SessionRepository, session_view};
use lens_contract::auth::{SessionRequest, SessionView};
use rand::RngCore;
use serde_json::{Value, json};

use crate::auth::{credentials, session_cookie};
use crate::error::SessionError;

pub fn router<R: SessionRepository + 'static>(authentication: Authentication<R>) -> Router {
    router_with_auth(Arc::new(authentication))
}

pub fn router_with_auth<R: SessionRepository + 'static>(
    authentication: Arc<Authentication<R>>,
) -> Router {
    Router::new()
        .route(
            "/auth/session",
            get(info::<R>).post(sign_in::<R>).delete(sign_out::<R>),
        )
        .with_state(authentication)
}

async fn sign_in<R: SessionRepository>(
    State(auth): State<Arc<Authentication<R>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, SessionError> {
    let request = request(&body, &headers)?;
    let mut bytes = [0u8; 48];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(SessionError::Entropy)?;
    let token = URL_SAFE_NO_PAD.encode(bytes);
    let view = auth.sign_in(&request.token, &token, Utc::now()).await?;
    let secure = if auth.settings.secure_cookie() {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!(
        "lens_session={token}; HttpOnly; Max-Age={}; Path=/; SameSite=strict{secure}",
        lens_auth::SESSION_LIFETIME_SECONDS
    );
    Ok(([(header::SET_COOKIE, cookie)], Json(view)).into_response())
}

async fn info<R: SessionRepository>(
    State(auth): State<Arc<Authentication<R>>>,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<SessionView>, SessionError> {
    let session = session_cookie(&headers);
    let identity = auth
        .authenticate(
            credentials(&headers, &method, session.as_deref())?,
            Utc::now(),
        )
        .await?;
    Ok(Json(session_view(identity)))
}

async fn sign_out<R: SessionRepository>(
    State(auth): State<Arc<Authentication<R>>>,
    headers: HeaderMap,
    method: Method,
) -> Result<Response, SessionError> {
    let session = session_cookie(&headers);
    auth.sign_out(
        credentials(&headers, &method, session.as_deref())?,
        Utc::now(),
    )
    .await?;
    let now = Utc::now().format("%a, %d %b %Y %H:%M:%S GMT");
    let cookie = format!("lens_session=\"\"; expires={now}; Max-Age=0; Path=/; SameSite=lax");
    Ok((StatusCode::NO_CONTENT, [(header::SET_COOKIE, cookie)]).into_response())
}

fn validation(kind: &str, path: Vec<Value>, message: &str, input: Value) -> Value {
    json!({"type": kind, "loc": path, "msg": message, "input": input})
}

fn request(body: &[u8], headers: &HeaderMap) -> Result<SessionRequest, SessionError> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    let json = content_type.is_none_or(|value| {
        let media_type = value
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase();
        media_type == "application/json"
            || (media_type.starts_with("application/") && media_type.ends_with("+json"))
    });
    let value: Value = if body.is_empty() {
        Value::Null
    } else if !json {
        Value::String(String::from_utf8_lossy(body).into_owned())
    } else {
        serde_json::from_slice(body).map_err(|error| SessionError::Validation(vec![json!({
            "type": "json_invalid", "loc": ["body", error.column()], "msg": "JSON decode error", "input": {}, "ctx": {"error": error.to_string()}
        })]))?
    };
    let Some(object) = value.as_object() else {
        let (kind, message) = if value.is_null() {
            ("missing", "Field required")
        } else {
            (
                "model_attributes_type",
                "Input should be a valid dictionary or object to extract fields from",
            )
        };
        return Err(SessionError::Validation(vec![validation(
            kind,
            vec![json!("body")],
            message,
            value,
        )]));
    };
    let token_error = match object.get("token") {
        Some(Value::String(_)) => None,
        Some(input) => Some(validation(
            "string_type",
            vec![json!("body"), json!("token")],
            "Input should be a valid string",
            input.clone(),
        )),
        None => Some(validation(
            "missing",
            vec![json!("body"), json!("token")],
            "Field required",
            value.clone(),
        )),
    };
    let errors: Vec<_> = token_error
        .into_iter()
        .chain(
            object
                .iter()
                .filter(|(key, _)| *key != "token")
                .map(|(key, value)| {
                    validation(
                        "extra_forbidden",
                        vec![json!("body"), json!(key)],
                        "Extra inputs are not permitted",
                        value.clone(),
                    )
                }),
        )
        .collect();
    if !errors.is_empty() {
        return Err(SessionError::Validation(errors));
    }
    serde_json::from_value(value).map_err(|_| SessionError::Validation(Vec::new()))
}
