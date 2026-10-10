use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, header},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken::{
    Algorithm, DecodingKey, Validation, decode, decode_header,
    jwk::{AlgorithmParameters, JwkSet, KeyAlgorithm, PublicKeyUse},
};
use lens_auth::{Authentication, SESSION_LIFETIME_SECONDS, SessionId, SessionRepository};
use lens_contract::auth::{Identity, Role};
use rand::RngCore;
use ring::aead;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use url::Url;

use crate::error::SignInError;

fn cookie_name(secure: bool) -> &'static str {
    if secure {
        "__Host-lens_google_login"
    } else {
        "lens_google_login"
    }
}
const TTL: Duration = Duration::from_secs(600);
const MAX_REDEEMED: usize = 1024;
const STATE_AAD: &[u8] = b"lens/google/login/v1";

pub struct GoogleConfig {
    client_id: String,
    client_secret: String,
    domains: Vec<String>,
    callback: String,
    secure: bool,
}

impl GoogleConfig {
    pub fn read(
        read: impl Fn(&str) -> Option<String>,
        public_url: &str,
    ) -> Result<Option<Self>, SignInError> {
        let id = read("LENS_GOOGLE_CLIENT_ID").unwrap_or_default();
        let secret = read("LENS_GOOGLE_CLIENT_SECRET").unwrap_or_default();
        let domains = read("LENS_GOOGLE_ALLOWED_DOMAINS").unwrap_or_default();
        if id.is_empty() && secret.is_empty() && domains.is_empty() {
            return Ok(None);
        }
        let domains: Vec<_> = domains
            .split(',')
            .map(|v| v.trim().to_ascii_lowercase())
            .collect();
        let origin = Url::parse(public_url).map_err(|_| SignInError)?;
        if id.trim().is_empty()
            || secret.trim().is_empty()
            || id.chars().any(char::is_whitespace)
            || secret.chars().any(char::is_whitespace)
            || !domains.iter().all(|domain| valid_domain(domain))
            || origin.host_str().is_none()
            || !origin.username().is_empty()
            || origin.password().is_some()
            || origin.path() != "/"
            || origin.query().is_some()
            || origin.fragment().is_some()
            || !(origin.scheme() == "https"
                || (origin.scheme() == "http"
                    && matches!(origin.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))))
        {
            return Err(SignInError);
        }
        Ok(Some(Self {
            client_id: id,
            client_secret: secret,
            domains,
            callback: origin
                .join("/auth/google/callback")
                .map_err(|_| SignInError)?
                .into(),
            secure: origin.scheme() == "https",
        }))
    }
}

fn valid_domain(value: &str) -> bool {
    value.len() <= 253
        && value.contains('.')
        && value.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

#[derive(Deserialize, Serialize)]
struct Pending {
    state: String,
    nonce: String,
    verifier: String,
    return_to: String,
    expires: u64,
}

struct Google<R> {
    auth: Arc<Authentication<R>>,
    config: Option<GoogleConfig>,
    client: reqwest::Client,
    token_url: String,
    keys_url: String,
    encryption: aead::LessSafeKey,
    started: Instant,
    redeemed: Mutex<HashMap<[u8; 32], u64>>,
    exchanges: tokio::sync::Semaphore,
}

pub fn router<R: SessionRepository + 'static>(
    auth: Arc<Authentication<R>>,
    config: Option<GoogleConfig>,
) -> Result<Router, SignInError> {
    let state = Google {
        auth,
        config,
        client: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| SignInError)?,
        token_url: "https://oauth2.googleapis.com/token".into(),
        keys_url: "https://www.googleapis.com/oauth2/v3/certs".into(),
        encryption: state_key()?,
        started: Instant::now(),
        redeemed: Mutex::new(HashMap::new()),
        exchanges: tokio::sync::Semaphore::new(8),
    };
    Ok(routes(Arc::new(state)))
}

fn routes<R: SessionRepository + 'static>(state: Arc<Google<R>>) -> Router {
    Router::new()
        .route("/auth/google/config", get(config::<R>))
        .route("/auth/google/start", get(start::<R>))
        .route("/auth/google/callback", get(callback::<R>))
        .with_state(state)
}

async fn config<R: SessionRepository>(State(state): State<Arc<Google<R>>>) -> Response {
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({"enabled": state.config.is_some()})),
    )
        .into_response()
}

#[derive(Default, Deserialize)]
struct Start {
    return_to: Option<String>,
}

fn destination(value: &str) -> String {
    let decoded = percent_encoding::percent_decode_str(value).decode_utf8();
    if value.len() > 2048
        || !value.starts_with('/')
        || value.starts_with("//")
        || decoded.as_ref().map_or(true, |v| {
            v.starts_with("//") || v.chars().any(|c| c == '\\' || c.is_control())
        })
        || value.chars().any(|c| c == '\\' || c.is_control())
    {
        return "/".into();
    }
    let base = Url::parse("https://lens.invalid/").expect("static URL");
    match base.join(value) {
        Ok(url) if url.origin() == base.origin() && !url.path().starts_with("/auth/") => {
            value.into()
        }
        _ => "/".into(),
    }
}

fn failed(target: &str) -> String {
    let (path, fragment) = target
        .split_once('#')
        .map_or((target, None), |(p, f)| (p, Some(f)));
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    let query = query
        .split('&')
        .filter(|part| !part.is_empty())
        .filter(|part| {
            url::form_urlencoded::parse(part.as_bytes())
                .next()
                .is_none_or(|(key, _)| key != "sso_error")
        })
        .collect::<Vec<_>>()
        .join("&");
    format!(
        "{path}?{}sso_error=failed{}",
        if query.is_empty() {
            String::new()
        } else {
            format!("{query}&")
        },
        fragment.map(|f| format!("#{f}")).unwrap_or_default()
    )
}

fn response(target: &str, secure: bool) -> Response {
    let mut response = Redirect::to(target).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert(header::REFERRER_POLICY, "no-referrer".parse().unwrap());
    response.headers_mut().append(
        header::SET_COOKIE,
        login_cookie("", secure, 0).parse().unwrap(),
    );
    response
}

fn login_cookie(value: &str, secure: bool, age: u64) -> String {
    format!(
        "{}={value}; Path={}; HttpOnly; SameSite=Lax; Max-Age={age}{}",
        cookie_name(secure),
        if secure { "/" } else { "/auth/google" },
        if secure { "; Secure" } else { "" }
    )
}

fn random() -> Result<String, SignInError> {
    let mut bytes = [0; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| SignInError)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn state_key() -> Result<aead::LessSafeKey, SignInError> {
    let mut key = [0; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut key)
        .map_err(|_| SignInError)?;
    Ok(aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, &key).map_err(|_| SignInError)?,
    ))
}

fn digest(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}

async fn start<R: SessionRepository>(
    State(state): State<Arc<Google<R>>>,
    query: Result<Query<Start>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let target = destination(
        query
            .ok()
            .and_then(|q| q.return_to.clone())
            .as_deref()
            .unwrap_or("/"),
    );
    let secure = state.config.as_ref().is_some_and(|c| c.secure);
    match state.begin(target.clone()) {
        Ok((url, browser)) => {
            let mut response = response(url.as_str(), secure);
            response.headers_mut().insert(
                header::SET_COOKIE,
                login_cookie(&browser, secure, TTL.as_secs())
                    .parse()
                    .unwrap(),
            );
            response
        }
        Err(_) => response(&failed(&target), secure),
    }
}

impl<R: SessionRepository> Google<R> {
    fn begin(&self, return_to: String) -> Result<(Url, String), SignInError> {
        let config = self.config.as_ref().ok_or(SignInError)?;
        let (state, nonce, verifier) = (random()?, random()?, random()?);
        let mut url =
            Url::parse("https://accounts.google.com/o/oauth2/v2/auth").map_err(|_| SignInError)?;
        url.query_pairs_mut().extend_pairs([
            ("client_id", config.client_id.as_str()),
            ("redirect_uri", &config.callback),
            ("response_type", "code"),
            ("scope", "openid email"),
            ("state", &state),
            ("nonce", &nonce),
            ("code_challenge", &URL_SAFE_NO_PAD.encode(digest(&verifier))),
            ("code_challenge_method", "S256"),
            ("prompt", "select_account"),
        ]);
        let mut pending = Pending {
            state,
            nonce,
            verifier,
            return_to,
            expires: self.started.elapsed().as_secs() + TTL.as_secs(),
        };
        let mut browser = self.seal(&pending)?;
        if browser.len() > 3800 {
            pending.return_to = "/".into();
            browser = self.seal(&pending)?;
        }
        Ok((url, browser))
    }

    fn seal(&self, pending: &Pending) -> Result<String, SignInError> {
        let mut nonce = [0; aead::NONCE_LEN];
        rand::rngs::OsRng
            .try_fill_bytes(&mut nonce)
            .map_err(|_| SignInError)?;
        let mut ciphertext = serde_json::to_vec(pending).map_err(|_| SignInError)?;
        self.encryption
            .seal_in_place_append_tag(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(STATE_AAD),
                &mut ciphertext,
            )
            .map_err(|_| SignInError)?;
        let mut envelope = nonce.to_vec();
        envelope.extend(ciphertext);
        Ok(URL_SAFE_NO_PAD.encode(envelope))
    }

    fn open_state(&self, state: &str, browser: &str) -> Result<Pending, SignInError> {
        if state.len() != 43 || browser.len() > 3800 {
            return Err(SignInError);
        }
        let mut envelope = URL_SAFE_NO_PAD.decode(browser).map_err(|_| SignInError)?;
        if envelope.len() < aead::NONCE_LEN + aead::AES_256_GCM.tag_len() {
            return Err(SignInError);
        }
        let (nonce, ciphertext) = envelope.split_at_mut(aead::NONCE_LEN);
        let plaintext = self
            .encryption
            .open_in_place(
                aead::Nonce::try_assume_unique_for_key(nonce).map_err(|_| SignInError)?,
                aead::Aad::from(STATE_AAD),
                ciphertext,
            )
            .map_err(|_| SignInError)?;
        let pending: Pending = serde_json::from_slice(plaintext).map_err(|_| SignInError)?;
        if !bool::from(pending.state.as_bytes().ct_eq(state.as_bytes()))
            || pending.expires <= self.started.elapsed().as_secs()
            || self
                .redeemed
                .lock()
                .map_err(|_| SignInError)?
                .contains_key(&digest(state))
        {
            return Err(SignInError);
        }
        Ok(pending)
    }

    fn redeem(&self, pending: &Pending) -> Result<(), SignInError> {
        let now = self.started.elapsed().as_secs();
        let mut redeemed = self.redeemed.lock().map_err(|_| SignInError)?;
        redeemed.retain(|_, expires| *expires > now);
        let key = digest(&pending.state);
        if pending.expires <= now || redeemed.contains_key(&key) || redeemed.len() >= MAX_REDEEMED {
            return Err(SignInError);
        }
        redeemed.insert(key, pending.expires);
        Ok(())
    }

    async fn exchange(&self, code: &str, pending: &Pending) -> Result<Identity, SignInError> {
        let config = self.config.as_ref().ok_or(SignInError)?;
        let _permit = self.exchanges.try_acquire().map_err(|_| SignInError)?;
        #[derive(Deserialize)]
        struct Tokens {
            id_token: String,
        }
        let tokens: Tokens = read_json(
            self.client
                .post(&self.token_url)
                .form(&[
                    ("grant_type", "authorization_code"),
                    ("code", code),
                    ("client_id", &config.client_id),
                    ("client_secret", &config.client_secret),
                    ("redirect_uri", &config.callback),
                    ("code_verifier", &pending.verifier),
                ])
                .send()
                .await
                .map_err(|_| SignInError)?,
        )
        .await?;
        if tokens.id_token.len() > 16000 {
            return Err(SignInError);
        }
        let header = decode_header(&tokens.id_token).map_err(|_| SignInError)?;
        if header.alg != Algorithm::RS256 {
            return Err(SignInError);
        }
        let kid = header.kid.ok_or(SignInError)?;
        let keys: JwkSet = read_json(
            self.client
                .get(&self.keys_url)
                .send()
                .await
                .map_err(|_| SignInError)?,
        )
        .await?;
        let key = keys.find(&kid).ok_or(SignInError)?;
        if key.common.public_key_use != Some(PublicKeyUse::Signature)
            || key.common.key_algorithm != Some(KeyAlgorithm::RS256)
            || !matches!(key.algorithm, AlgorithmParameters::RSA(_))
        {
            return Err(SignInError);
        }
        let key = DecodingKey::from_jwk(key).map_err(|_| SignInError)?;
        verify(&tokens.id_token, &key, config, &pending.nonce)
    }
}

async fn read_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, SignInError> {
    if !response.status().is_success() {
        return Err(SignInError);
    }
    let mut response = response;
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| SignInError)? {
        if body.len() + chunk.len() > 65536 {
            return Err(SignInError);
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| SignInError)
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}

#[derive(Clone, Deserialize)]
struct Claims {
    sub: String,
    aud: Audience,
    azp: Option<String>,
    iat: i64,
    exp: i64,
    nonce: String,
    email: String,
    email_verified: bool,
    hd: String,
}

fn verify(
    token: &str,
    key: &DecodingKey,
    config: &GoogleConfig,
    nonce: &str,
) -> Result<Identity, SignInError> {
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&["https://accounts.google.com", "accounts.google.com"]);
    validation.set_audience(&[&config.client_id]);
    validation.set_required_spec_claims(&["exp", "iat", "iss", "aud", "sub"]);
    validation.leeway = 30;
    validation.validate_nbf = true;
    let claims = decode::<Claims>(token, key, &validation)
        .map_err(|_| SignInError)?
        .claims;
    let now = chrono::Utc::now().timestamp();
    let multiple = match &claims.aud {
        Audience::Many(values) => values.len() > 1,
        Audience::One(value) => {
            if value.is_empty() {
                return Err(SignInError);
            }
            false
        }
    };
    let domain = claims.hd.to_ascii_lowercase();
    let email_domain = claims
        .email
        .rsplit_once('@')
        .filter(|(local, _)| !local.is_empty() && !local.contains('@'))
        .map(|(_, domain)| domain.to_ascii_lowercase());
    if claims.sub.is_empty()
        || claims.sub.len() > 255
        || claims.sub.chars().any(char::is_control)
        || claims.iat > now + 30
        || claims.iat > claims.exp
        || claims.exp <= now - 30
        || !bool::from(claims.nonce.as_bytes().ct_eq(nonce.as_bytes()))
        || claims
            .azp
            .as_ref()
            .is_some_and(|azp| azp != &config.client_id)
        || (multiple && claims.azp.is_none())
        || !claims.email_verified
        || !config.domains.contains(&domain)
        || email_domain.as_deref() != Some(domain.as_str())
        || claims
            .email
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(SignInError);
    }
    Ok(Identity {
        user_id: Some(format!("google:{}", claims.sub)),
        user_role: Role::ProxyAdminViewer,
        ..Identity::default()
    })
}

#[derive(Default, Deserialize)]
struct Callback {
    state: Option<String>,
    code: Option<String>,
    error: Option<String>,
}

async fn callback<R: SessionRepository>(
    State(state): State<Arc<Google<R>>>,
    headers: HeaderMap,
    query: Result<Query<Callback>, axum::extract::rejection::QueryRejection>,
) -> Response {
    let secure = state.config.as_ref().is_some_and(|c| c.secure);
    let query = query.map(|q| q.0).unwrap_or_default();
    let browsers: Vec<_> = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .flat_map(cookie::Cookie::split_parse)
        .filter_map(Result::ok)
        .filter(|c| c.name() == cookie_name(secure))
        .map(|c| c.value().to_owned())
        .collect();
    let pending = if browsers.len() == 1 {
        state
            .open_state(query.state.as_deref().unwrap_or_default(), &browsers[0])
            .ok()
    } else {
        None
    };
    let Some(pending) = pending else {
        return response(&failed("/"), secure);
    };
    let failure = response(&failed(&pending.return_to), secure);
    let Some(code) = query
        .code
        .filter(|code| !code.is_empty() && code.len() <= 4096)
    else {
        return failure;
    };
    if query.error.is_some() {
        return failure;
    }
    let Ok(identity) = state.exchange(&code, &pending).await else {
        return failure;
    };
    if state.redeem(&pending).is_err() {
        return failure;
    }
    let Ok(token) = random() else {
        return failure;
    };
    if state
        .auth
        .sessions
        .create_identity(
            &SessionId::for_token(&token),
            chrono::Utc::now() + chrono::Duration::seconds(SESSION_LIFETIME_SECONDS),
            &identity,
        )
        .await
        .is_err()
    {
        return failure;
    }
    let mut response = response(&pending.return_to, secure);
    response.headers_mut().append(header::SET_COOKIE, format!("lens_session={token}; HttpOnly; Max-Age={SESSION_LIFETIME_SECONDS}; Path=/; SameSite=Lax{}", if secure { "; Secure" } else { "" }).parse().unwrap());
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tracing::{TraceAgent, TraceConfig, TraceReadError, TraceReader};
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use chrono::{DateTime, Utc};
    use jsonwebtoken::{EncodingKey, Header, encode};
    use lens_auth::{Credentials, StoreError};
    use litellm_traces::{
        QueryScope, SpanDetail, SpanErrorPage, Trace, TraceConversationPage, TracePage,
        query::named::ReadAccessParams,
        request::{
            TraceConversationRequest, TraceDetailRequest, TraceErrorPageRequest, TraceSpanRequest,
        },
    };

    use rstest::{fixture, rstest};
    use serde_json::{Value, json};
    use tower::ServiceExt;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_string_contains, method, path},
    };

    struct SharedTraces;
    impl TraceReader for SharedTraces {
        type Help = Value;
        async fn list(
            &self,
            _scope: ReadAccessParams,
            _start_ms: i64,
            _end_ms: i64,
            _cursor: Option<String>,
            _limit: u32,
        ) -> Result<TracePage, TraceReadError> {
            unreachable!("unexpected trace list")
        }
        async fn agents(
            &self,
            _scope: ReadAccessParams,
            _start_ms: i64,
            _end_ms: i64,
            _limit: u32,
        ) -> Result<Vec<TraceAgent>, TraceReadError> {
            unreachable!("unexpected trace agents")
        }
        async fn trace(
            &self,
            _scope: ReadAccessParams,
            _trace_id: String,
            _request: TraceDetailRequest,
        ) -> Result<Option<Trace>, TraceReadError> {
            unreachable!("unexpected trace detail")
        }
        async fn span(
            &self,
            _scope: ReadAccessParams,
            _trace_id: String,
            _span_id: String,
            _request: TraceSpanRequest,
        ) -> Result<Option<SpanDetail>, TraceReadError> {
            unreachable!("unexpected trace detail")
        }
        async fn conversation(
            &self,
            _scope: ReadAccessParams,
            _trace_id: String,
            _request: TraceConversationRequest,
        ) -> Result<Option<TraceConversationPage>, TraceReadError> {
            unreachable!("unexpected trace detail")
        }
        async fn span_error(
            &self,
            _scope: ReadAccessParams,
            _trace_id: String,
            _span_id: String,
            _request: TraceErrorPageRequest,
        ) -> Result<Option<SpanErrorPage>, TraceReadError> {
            unreachable!("unexpected trace detail")
        }
        async fn query(&self, scope: QueryScope, _sql: String) -> Result<Value, TraceReadError> {
            Ok(if matches!(scope, QueryScope::All) {
                json!({"data":[{"trace_id":"shared-trace", "user_id":"another-user", "team_id":"another-team"}]})
            } else {
                json!({"data":[]})
            })
        }
        async fn help(&self, _scope: QueryScope) -> Result<Value, TraceReadError> {
            unreachable!("unexpected trace help")
        }
    }

    type Stored = HashMap<String, (DateTime<Utc>, Identity)>;
    #[derive(Default)]
    struct Memory(Mutex<Stored>);
    impl SessionRepository for Memory {
        async fn create(&self, id: &SessionId, expires: DateTime<Utc>) -> Result<(), StoreError> {
            self.create_identity(id, expires, &lens_auth::local_admin())
                .await
        }
        async fn create_identity(
            &self,
            id: &SessionId,
            expires: DateTime<Utc>,
            identity: &Identity,
        ) -> Result<(), StoreError> {
            let mut values = self.0.lock().unwrap();
            if values.contains_key(id.as_str()) {
                return Err(StoreError::Conflict);
            }
            values.insert(id.as_str().into(), (expires, identity.clone()));
            Ok(())
        }
        async fn expires_at(&self, id: &SessionId) -> Result<Option<DateTime<Utc>>, StoreError> {
            Ok(self.0.lock().unwrap().get(id.as_str()).map(|v| v.0))
        }
        async fn identity(&self, id: &SessionId) -> Result<Option<Identity>, StoreError> {
            Ok(self.0.lock().unwrap().get(id.as_str()).map(|v| v.1.clone()))
        }
        async fn revoke(&self, id: &SessionId) -> Result<(), StoreError> {
            self.0.lock().unwrap().remove(id.as_str());
            Ok(())
        }
    }

    fn settings(
        id: &str,
        secret: &str,
        domains: &str,
        origin: &str,
    ) -> Result<Option<GoogleConfig>, SignInError> {
        GoogleConfig::read(
            |key| match key {
                "LENS_GOOGLE_CLIENT_ID" => Some(id.into()),
                "LENS_GOOGLE_CLIENT_SECRET" => Some(secret.into()),
                "LENS_GOOGLE_ALLOWED_DOMAINS" => Some(domains.into()),
                _ => None,
            },
            origin,
        )
    }

    #[fixture]
    fn config() -> GoogleConfig {
        settings("client", "secret", "example.com", "https://lens.test")
            .unwrap()
            .unwrap()
    }

    #[fixture]
    fn state(config: GoogleConfig) -> Arc<Google<Memory>> {
        Arc::new(Google {
            auth: Arc::new(Authentication {
                settings: lens_auth::Settings::new(
                    "test-admin-token-at-least-32-characters",
                    None,
                    "https://lens.test",
                )
                .unwrap(),
                sessions: Memory::default(),
            }),
            config: Some(config),
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            token_url: String::new(),
            keys_url: String::new(),
            encryption: state_key().unwrap(),
            started: Instant::now(),
            redeemed: Mutex::new(HashMap::new()),
            exchanges: tokio::sync::Semaphore::new(8),
        })
    }

    #[rstest]
    #[case::disabled("", "", "", "https://lens.test", true, false)]
    #[case::partial("client", "", "example.com", "https://lens.test", false, false)]
    #[case::no_domains("client", "secret", "", "https://lens.test", false, false)]
    #[case::wildcard("client", "secret", "*.example.com", "https://lens.test", false, false)]
    #[case::trailing_comma("client", "secret", "example.com,", "https://lens.test", false, false)]
    #[case::http_remote("client", "secret", "example.com", "http://lens.test", false, false)]
    #[case::credentials(
        "client",
        "secret",
        "example.com",
        "https://user@lens.test",
        false,
        false
    )]
    #[case::path(
        "client",
        "secret",
        "example.com",
        "https://lens.test/prefix",
        false,
        false
    )]
    #[case::query(
        "client",
        "secret",
        "example.com",
        "https://lens.test/?a=1",
        false,
        false
    )]
    #[case::fragment(
        "client",
        "secret",
        "example.com",
        "https://lens.test/#a",
        false,
        false
    )]
    #[case::local("client", "secret", "example.com", "http://localhost:4318", true, true)]
    #[case::allowed(
        "client",
        "secret",
        " EXAMPLE.COM, other.test ",
        "https://lens.test",
        true,
        true
    )]
    fn configuration(
        #[case] id: &str,
        #[case] secret: &str,
        #[case] domains: &str,
        #[case] origin: &str,
        #[case] valid: bool,
        #[case] enabled: bool,
    ) {
        let result = settings(id, secret, domains, origin);
        assert_eq!(result.is_ok(), valid);
        if valid {
            assert_eq!(result.unwrap().is_some(), enabled);
        }
    }

    #[rstest]
    #[case::path("/traces?a=1#span", "/traces?a=1#span")]
    #[case::absolute("https://evil.test/", "/")]
    #[case::network("//evil.test/", "/")]
    #[case::backslash("/\\evil.test/", "/")]
    #[case::encoded("/%2fevil.test/", "/")]
    #[case::encoded_backslash("/%5cevil.test/", "/")]
    #[case::control("/x%0d%0aLocation:evil", "/")]
    #[case::loopback("/auth/google/start", "/")]
    #[case::dot_segments("/x/../auth/google/start", "/")]
    fn return_paths(#[case] input: &str, #[case] expected: &str) {
        assert_eq!(destination(input), expected);
    }

    #[rstest]
    fn failure_preserves_query_and_fragment() {
        assert_eq!(
            failed("/traces?a=1#span"),
            "/traces?a=1&sso_error=failed#span"
        );
    }

    #[rstest]
    fn failure_replaces_existing_error() {
        assert_eq!(
            failed("/traces?sso_error=old&a=1&%73so_error=other#span"),
            "/traces?a=1&sso_error=failed#span"
        );
    }

    #[rstest]
    fn rejects_invalid_signature(config: GoogleConfig, claims: Value) {
        let token = signed(&claims);
        let (data, signature) = token.rsplit_once('.').unwrap();
        let mut bytes = URL_SAFE_NO_PAD.decode(signature).unwrap();
        bytes[0] ^= 1;
        let tampered = format!("{data}.{}", URL_SAFE_NO_PAD.encode(bytes));
        assert!(verify(&tampered, &public_key(), &config, "nonce").is_err());
    }

    #[rstest]
    #[tokio::test]
    async fn disabled_routes(state: Arc<Google<Memory>>) {
        let app = router(state.auth.clone(), None).unwrap();
        let response = app
            .clone()
            .oneshot(request("/auth/google/config", None))
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&to_bytes(response.into_body(), 1024).await.unwrap())
                .unwrap(),
            json!({"enabled":false})
        );
        let response = app
            .oneshot(request(
                "/auth/google/start?return_to=%2Ftraces%23span",
                None,
            ))
            .await
            .unwrap();
        assert_eq!(
            response.headers()[header::LOCATION],
            "/traces?sso_error=failed#span"
        );
        assert!(state.auth.sessions.0.lock().unwrap().is_empty());
    }

    #[rstest]
    fn single_use_and_browser_binding(state: Arc<Google<Memory>>) {
        let (url, browser) = state.begin("/traces".into()).unwrap();
        let pairs: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert!(
            state
                .open_state(&pairs["state"], &random().unwrap())
                .is_err()
        );
        let pending = state.open_state(&pairs["state"], &browser).unwrap();
        assert_eq!(
            pairs["code_challenge"],
            URL_SAFE_NO_PAD.encode(digest(&pending.verifier))
        );
        assert_eq!(pairs["code_challenge_method"], "S256");
        assert_eq!(pending.nonce, pairs["nonce"]);
        state.redeem(&pending).unwrap();
        assert!(state.redeem(&pending).is_err());
        assert!(state.open_state(&pairs["state"], &browser).is_err());
    }

    #[rstest]
    #[case::before_expiry(599, true)]
    #[case::at_expiry(600, false)]
    #[case::after_expiry(601, false)]
    fn state_expiry(mut state: Arc<Google<Memory>>, #[case] elapsed: u64, #[case] valid: bool) {
        let (url, browser) = state.begin("/".into()).unwrap();
        let pairs: HashMap<_, _> = url.query_pairs().into_owned().collect();
        let pending = state.open_state(&pairs["state"], &browser).unwrap();
        Arc::get_mut(&mut state).unwrap().started = Instant::now() - Duration::from_secs(elapsed);
        assert_eq!(state.open_state(&pairs["state"], &browser).is_ok(), valid);
        assert_eq!(state.redeem(&pending).is_ok(), valid);
    }

    #[rstest]
    #[case::nonce(0)]
    #[case::ciphertext(aead::NONCE_LEN)]
    #[case::tag(usize::MAX)]
    fn tampered_browser_state(state: Arc<Google<Memory>>, #[case] offset: usize) {
        let (url, browser) = state.begin("/traces".into()).unwrap();
        let pairs: HashMap<_, _> = url.query_pairs().into_owned().collect();
        let mut envelope = URL_SAFE_NO_PAD.decode(&browser).unwrap();
        let index = offset.min(envelope.len() - 1);
        envelope[index] ^= 1;
        assert!(
            state
                .open_state(&pairs["state"], &URL_SAFE_NO_PAD.encode(envelope))
                .is_err()
        );
        assert!(state.open_state(&pairs["state"], &browser).is_ok());
    }

    #[rstest]
    fn other_login_cookie_does_not_bind(state: Arc<Google<Memory>>) {
        let (url, browser) = state.begin("/traces".into()).unwrap();
        let pairs: HashMap<_, _> = url.query_pairs().into_owned().collect();
        let (_, other_browser) = state.begin("/traces".into()).unwrap();
        assert!(state.open_state(&pairs["state"], &other_browser).is_err());
        assert!(state.open_state(&pairs["state"], &browser).is_ok());
    }

    #[rstest]
    fn restart_invalidates_state(mut state: Arc<Google<Memory>>) {
        let (url, browser) = state.begin("/traces".into()).unwrap();
        let pairs: HashMap<_, _> = url.query_pairs().into_owned().collect();
        Arc::get_mut(&mut state).unwrap().encryption = state_key().unwrap();
        assert!(state.open_state(&pairs["state"], &browser).is_err());
    }

    #[rstest]
    fn concurrent_redemption_is_single_use(state: Arc<Google<Memory>>) {
        let (url, browser) = state.begin("/".into()).unwrap();
        let pairs: HashMap<_, _> = url.query_pairs().into_owned().collect();
        let pending = state.open_state(&pairs["state"], &browser).unwrap();
        let barrier = std::sync::Barrier::new(2);
        let results = std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                barrier.wait();
                state.redeem(&pending).is_ok()
            });
            let second = scope.spawn(|| {
                barrier.wait();
                state.redeem(&pending).is_ok()
            });
            (first.join().unwrap(), second.join().unwrap())
        });
        assert_ne!(results.0, results.1);
        assert!(state.open_state(&pairs["state"], &browser).is_err());
    }

    #[rstest]
    fn large_destination_still_fits_browser_cookie(state: Arc<Google<Memory>>) {
        let (url, browser) = state.begin(format!("/{}", "\"".repeat(2047))).unwrap();
        let pairs: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert!(login_cookie(&browser, true, TTL.as_secs()).len() < 4096);
        assert_eq!(
            state
                .open_state(&pairs["state"], &browser)
                .unwrap()
                .return_to,
            "/"
        );
    }

    async fn abandon_starts(app: &Router, count: usize) -> usize {
        let mut accepted = 0;
        for _ in 0..count {
            let response = app
                .clone()
                .oneshot(request("/auth/google/start", None))
                .await
                .unwrap();
            if response.headers()[header::LOCATION]
                .to_str()
                .unwrap()
                .starts_with("https://accounts.google.com/")
            {
                accepted += 1;
            }
        }
        accepted
    }

    #[fixture]
    fn claims() -> Value {
        json!({"iss":"https://accounts.google.com", "aud":"client", "sub":"subject-1", "exp":Utc::now().timestamp()+600, "iat":Utc::now().timestamp(), "nonce":"nonce", "email":"user@example.com", "email_verified":true, "hd":"example.com"})
    }
    fn signed(claims: &Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("test-key".into());
        encode(
            &header,
            claims,
            &EncodingKey::from_rsa_pem(include_bytes!("../tests/fixtures/github-test-key.pem"))
                .unwrap(),
        )
        .unwrap()
    }
    fn public_key() -> DecodingKey {
        let keys: JwkSet =
            serde_json::from_str(include_str!("../tests/fixtures/google-test-jwks.json")).unwrap();
        DecodingKey::from_jwk(&keys.keys[0]).unwrap()
    }

    #[rstest]
    #[case::issuer("iss", json!("https://evil.test"))]
    #[case::audience("aud", json!("other-client"))]
    #[case::authorized_party("azp", json!("other-client"))]
    #[case::missing_authorized_party("aud", json!(["client", "other"]))]
    #[case::unverified("email_verified", json!(false))]
    #[case::unverified_type("email_verified", json!("true"))]
    #[case::no_domain("hd", Value::Null)]
    #[case::bad_domain("hd", json!("evil.test"))]
    #[case::domain_suffix("hd", json!("sub.example.com"))]
    #[case::email_domain("email", json!("user@evil.test"))]
    #[case::empty_subject("sub", json!(""))]
    #[case::wrong_nonce("nonce", json!("wrong"))]
    #[case::expired("exp", json!(1))]
    #[case::future_issue("iat", json!(Utc::now().timestamp()+300))]
    #[case::missing_issue("iat", Value::Null)]
    #[case::future_not_before("nbf", json!(Utc::now().timestamp()+300))]
    fn rejected_claims(
        config: GoogleConfig,
        mut claims: Value,
        #[case] field: &str,
        #[case] value: Value,
    ) {
        claims[field] = value;
        assert!(verify(&signed(&claims), &public_key(), &config, "nonce").is_err());
    }

    #[rstest]
    #[case::single(json!("client"), Value::Null, "https://accounts.google.com")]
    #[case::multiple(json!(["client", "other"]), json!("client"), "accounts.google.com")]
    fn accepted_claims(
        config: GoogleConfig,
        mut claims: Value,
        #[case] audience: Value,
        #[case] azp: Value,
        #[case] issuer: &str,
    ) {
        claims["aud"] = audience;
        claims["azp"] = azp;
        claims["iss"] = json!(issuer);
        let identity = verify(&signed(&claims), &public_key(), &config, "nonce").unwrap();
        assert_eq!(identity.user_id.as_deref(), Some("google:subject-1"));
        assert_eq!(identity.user_role, Role::ProxyAdminViewer);
        assert!(identity.team_id.is_none() && identity.log_team_ids.is_empty());
        assert!(matches!(
            lens_auth::trace_read_scope(&identity),
            Some(lens_auth::ReadScope::AllRows)
        ));
    }

    #[rstest]
    fn rejects_algorithm_confusion(config: GoogleConfig, claims: Value) {
        let token = encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(include_bytes!("../tests/fixtures/google-test-jwks.json")),
        )
        .unwrap();
        assert!(verify(&token, &public_key(), &config, "nonce").is_err());
    }

    fn request(uri: &str, browser: Option<&str>) -> Request<Body> {
        let builder = Request::builder().uri(uri);
        let builder = if let Some(browser) = browser {
            builder.header(header::COOKIE, format!("{}={browser}", cookie_name(true)))
        } else {
            builder
        };
        builder.body(Body::empty()).unwrap()
    }

    #[rstest]
    #[case::normal(0)]
    #[case::abandoned_starts(2048)]
    #[tokio::test]
    async fn successful_flow_reads_shared_traces_without_admin_writes(
        mut state: Arc<Google<Memory>>,
        mut claims: Value,
        #[case] abandoned: usize,
    ) {
        let provider = MockServer::start().await;
        let inner = Arc::get_mut(&mut state).unwrap();
        inner.token_url = format!("{}/token", provider.uri());
        inner.keys_url = format!("{}/keys", provider.uri());
        let app = routes(state.clone())
            .merge(crate::sessions::router_with_auth(state.auth.clone()))
            .merge(crate::tracing::router(
                state.auth.clone(),
                SharedTraces,
                TraceConfig::default(),
            ))
            .merge(crate::models::router(state.auth.clone(), Vec::new()));
        let config = app
            .clone()
            .oneshot(request("/auth/google/config", None))
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&to_bytes(config.into_body(), 1024).await.unwrap())
                .unwrap(),
            json!({"enabled":true})
        );
        let start = app
            .clone()
            .oneshot(request(
                "/auth/google/start?return_to=%2Ftraces%3Fa%3D1%23span",
                None,
            ))
            .await
            .unwrap();
        assert_eq!(start.status(), StatusCode::SEE_OTHER);
        let cookie = start.headers()[header::SET_COOKIE].to_str().unwrap();
        assert!(cookie.starts_with("__Host-lens_google_login="));
        assert!(cookie.contains("Path=/;") && !cookie.contains("Domain="));
        assert!(
            cookie.contains("HttpOnly")
                && cookie.contains("Secure")
                && cookie.contains("SameSite=Lax")
        );
        let browser = cookie::Cookie::parse(cookie).unwrap().value().to_owned();
        let url = Url::parse(start.headers()[header::LOCATION].to_str().unwrap()).unwrap();
        let pairs: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(abandon_starts(&app, abandoned).await, abandoned);
        assert!(state.redeemed.lock().unwrap().is_empty());
        assert_eq!(abandon_starts(&app, 1).await, 1);
        let verifier = state
            .open_state(&pairs["state"], &browser)
            .unwrap()
            .verifier;
        claims["nonce"] = json!(pairs["nonce"]);
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("code=good-code"))
            .and(body_string_contains(format!("code_verifier={verifier}")))
            .and(body_string_contains("client_secret=secret"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"id_token":signed(&claims)})),
            )
            .expect(1)
            .mount(&provider)
            .await;
        Mock::given(method("GET"))
            .and(path("/keys"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                include_str!("../tests/fixtures/google-test-jwks.json"),
                "application/json",
            ))
            .expect(1)
            .mount(&provider)
            .await;
        let uri = format!(
            "/auth/google/callback?state={}&code=good-code",
            pairs["state"]
        );
        let missing_cookie = app.clone().oneshot(request(&uri, None)).await.unwrap();
        assert_eq!(
            missing_cookie.headers()[header::LOCATION],
            "/?sso_error=failed"
        );
        let callback = app
            .clone()
            .oneshot(request(&uri, Some(&browser)))
            .await
            .unwrap();
        assert_eq!(callback.headers()[header::LOCATION], "/traces?a=1#span");
        assert_eq!(callback.headers()[header::CACHE_CONTROL], "no-store");
        let session_cookie = callback
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|h| h.to_str().ok())
            .find(|c| c.starts_with("lens_session="))
            .unwrap();
        assert!(session_cookie.contains("HttpOnly") && session_cookie.contains("Secure"));
        let token = cookie::Cookie::parse(session_cookie)
            .unwrap()
            .value()
            .to_owned();
        let info = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/auth/session")
                    .header(header::COOKIE, format!("lens_session={token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(info.status(), StatusCode::OK);
        assert_eq!(
            serde_json::from_slice::<Value>(&to_bytes(info.into_body(), 1024).await.unwrap())
                .unwrap(),
            json!({"user_id":"google:subject-1", "user_role":"proxy_admin_viewer"})
        );
        let shared = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/traces/query")
                    .header(header::COOKIE, format!("lens_session={token}"))
                    .header(header::ORIGIN, "https://lens.test")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"sql":"SELECT TraceId AS trace_id, UserId AS user_id, TeamId AS team_id FROM otel_traces"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(shared.status(), StatusCode::OK);
        assert_eq!(
            serde_json::from_slice::<Value>(&to_bytes(shared.into_body(), 4096).await.unwrap())
                .unwrap(),
            json!({"data":[{"trace_id":"shared-trace", "user_id":"another-user", "team_id":"another-team"}]})
        );
        let denied = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/lens/gateway/refresh")
                    .header(header::COOKIE, format!("lens_session={token}"))
                    .header(header::ORIGIN, "https://lens.test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            serde_json::from_slice::<Value>(&to_bytes(denied.into_body(), 4096).await.unwrap())
                .unwrap(),
            json!({"detail": "Only proxy admins can configure or run Lens"})
        );
        let admin = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/lens/gateway/refresh")
                    .header(
                        header::AUTHORIZATION,
                        "Bearer test-admin-token-at-least-32-characters",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(admin.status(), StatusCode::OK);
        let credentials = || Credentials {
            authorization: None,
            session: Some(&token),
            method: "GET",
            origin: None,
        };
        let identity = state
            .auth
            .authenticate(credentials(), Utc::now())
            .await
            .unwrap();
        assert_eq!(identity.user_role, Role::ProxyAdminViewer);
        assert_eq!(identity.user_id.as_deref(), Some("google:subject-1"));
        assert!(
            state
                .auth
                .authenticate(
                    Credentials {
                        method: "POST",
                        ..credentials()
                    },
                    Utc::now()
                )
                .await
                .is_err()
        );
        let replay = app.oneshot(request(&uri, Some(&browser))).await.unwrap();
        assert_eq!(replay.headers()[header::LOCATION], "/?sso_error=failed");
        state
            .auth
            .sign_out(
                Credentials {
                    method: "DELETE",
                    origin: Some("https://lens.test"),
                    ..credentials()
                },
                Utc::now(),
            )
            .await
            .unwrap();
        assert!(
            state
                .auth
                .authenticate(credentials(), Utc::now())
                .await
                .is_err()
        );
        provider.verify().await;
    }

    #[rstest]
    #[case::denied("error=access_denied")]
    #[case::missing_code("")]
    #[case::provider_failure("code=bad-code")]
    #[tokio::test]
    async fn failure_is_generic_and_does_not_reserve_state(
        mut state: Arc<Google<Memory>>,
        #[case] query: &str,
    ) {
        let provider = MockServer::start().await;
        Arc::get_mut(&mut state).unwrap().token_url = provider.uri();
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_string("private-provider-detail"))
            .mount(&provider)
            .await;
        let (url, browser) = state.begin("/traces?a=1#span".into()).unwrap();
        let pairs: HashMap<_, _> = url.query_pairs().into_owned().collect();
        let response = routes(state.clone())
            .oneshot(request(
                &format!("/auth/google/callback?state={}&{query}", pairs["state"]),
                Some(&browser),
            ))
            .await
            .unwrap();
        assert_eq!(
            response.headers()[header::LOCATION],
            "/traces?a=1&sso_error=failed#span"
        );
        assert!(
            !response
                .headers()
                .get_all(header::SET_COOKIE)
                .iter()
                .any(|h| h.to_str().unwrap().starts_with("lens_session="))
        );
        assert!(state.redeemed.lock().unwrap().is_empty());
        assert!(state.auth.sessions.0.lock().unwrap().is_empty());
    }
}
