use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Form, Json,
    extract::{Path, State},
    http::{HeaderMap, header},
    response::{Html, IntoResponse, Redirect, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{TimeDelta, Utc};
use lens_auth::SessionRepository;
use lens_contract::github::{
    Authorization, AuthorizationState, BrokerConnection, BrokerHandshake, BrokerHandshakeState,
    Connection, Owner,
};
use litellm_storage_clickhouse::github::{StoredBrokerConnection, StoredHandshake};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use super::remote::{CALLBACK_PATH, RemoteCredentials, secure_origin};
use super::report::{Published, ReportInput};
use super::{App, GitHubApp, GitHubError, STATE_COOKIE, cookie_named, cookie_state, load};

const BROWSER_COOKIE: &str = "lens_github_service";

pub(super) struct Admission(Mutex<(Instant, u32)>);

impl Default for Admission {
    fn default() -> Self {
        Self(Mutex::new((Instant::now(), 0)))
    }
}

impl Admission {
    fn admit(&self, now: Instant) -> Result<(), GitHubError> {
        let mut window = self.0.lock().expect("admission lock");
        if now.duration_since(window.0) >= Duration::from_secs(60) {
            *window = (now, 0);
        }
        if window.1 >= 30 {
            return Err(GitHubError::RateLimited);
        }
        window.1 += 1;
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HandoffRequest {
    pub redirect_uri: String,
    pub state: String,
    pub code_challenge: String,
    pub agent: String,
    #[serde(default)]
    pub install: bool,
}

#[derive(Serialize, Deserialize)]
pub(super) struct HandoffResponse {
    pub authorization_url: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RedeemRequest {
    pub code: String,
    pub code_verifier: String,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Redeemed {
    pub connection: Connection,
    pub lens_origin: String,
    pub credential: RemoteCredentials,
}

#[derive(Serialize, Deserialize)]
pub(super) struct ServiceStatus {
    pub connection: Connection,
    pub app_slug: String,
    pub available: bool,
    pub availability_error: String,
}

fn enabled<R>(app: &App<R>) -> Result<&GitHubApp, GitHubError> {
    if !app.settings.service_enabled {
        return Err(GitHubError::NotConfigured);
    }
    app.settings
        .github
        .as_ref()
        .ok_or(GitHubError::NotConfigured)
}

fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value))
}

pub(super) async fn start<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    Json(request): Json<HandoffRequest>,
) -> Result<Json<HandoffResponse>, GitHubError> {
    enabled(&app)?;
    super::agent(&request.agent)?;
    if request.redirect_uri.len() > 2048 {
        return Err(GitHubError::Invalid("Invalid Lens callback URL"));
    }
    let redirect_uri = Url::parse(&request.redirect_uri)
        .map_err(|_| GitHubError::Invalid("Invalid Lens callback URL"))?;
    if !secure_origin(&redirect_uri)
        || redirect_uri.path() != CALLBACK_PATH
        || request.state.is_empty()
        || request.state.len() > 256
        || request.code_challenge.len() != 43
        || URL_SAFE_NO_PAD.decode(&request.code_challenge).is_err()
        || URL_SAFE_NO_PAD
            .decode(&request.code_challenge)
            .is_ok_and(|value| value.len() != 32)
    {
        return Err(GitHubError::Invalid("Invalid Lens connection request"));
    }
    app.admission.admit(Instant::now())?;
    let id = uuid::Uuid::new_v4().to_string();
    let expires_at = Utc::now() + TimeDelta::minutes(10);
    let authorization = Authorization {
        id: id.clone(),
        owner: Owner {
            scope: format!("github-service:{id}"),
            subject: id.clone(),
        },
        agent: request.agent.clone(),
        expires_at,
        state: if request.install {
            AuthorizationState::Installing
        } else {
            AuthorizationState::Pending
        },
    };
    let handshake = BrokerHandshake {
        id: id.clone(),
        browser_claimed: false,
        browser_hash: String::new(),
        lens_origin: redirect_uri.origin().ascii_serialization(),
        redirect_uri: redirect_uri.into(),
        local_state: request.state,
        code_challenge: request.code_challenge,
        agent: request.agent,
        expires_at,
        state: BrokerHandshakeState::Pending,
    };
    app.store
        .create_handshake(&authorization, &handshake)
        .await?;
    Ok(Json(HandoffResponse {
        authorization_url: app
            .settings
            .public_url
            .join(&format!("/lens/github/service/connect/{id}"))
            .expect("fixed path")
            .into(),
    }))
}

async fn handshake<R>(app: &App<R>, id: &str) -> Result<StoredHandshake, GitHubError> {
    uuid::Uuid::parse_str(id).map_err(|_| GitHubError::Expired)?;
    let stored = app.store.handshake(id).await?.ok_or(GitHubError::Expired)?;
    if stored.handshake.expires_at <= Utc::now() {
        return Err(GitHubError::Expired);
    }
    Ok(stored)
}

pub(super) fn redirect_to_consent(public_url: &Url, id: &str) -> Response {
    let url = public_url
        .join(&format!("/lens/github/service/connect/{id}"))
        .expect("fixed path");
    (
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        Redirect::to(url.as_str()),
    )
        .into_response()
}

pub(super) async fn browser<R>(
    app: &App<R>,
    headers: &HeaderMap,
    id: &str,
) -> Result<(), GitHubError> {
    let github = enabled(app)?;
    let state = cookie_state(headers).ok_or(GitHubError::Forbidden)?;
    if github.authorization_id(&state)? != id {
        return Err(GitHubError::Forbidden);
    }
    let stored = handshake(app, id).await?;
    let secret = cookie_named(headers, BROWSER_COOKIE).ok_or(GitHubError::Forbidden)?;
    if !stored.handshake.browser_claimed
        || secret.len() != 64
        || hash(&secret) != stored.handshake.browser_hash
    {
        return Err(GitHubError::Forbidden);
    }
    Ok(())
}

fn cookie(github: &GitHubApp, id: &str) -> String {
    cookie::Cookie::build((STATE_COOKIE, github.state(id)))
        .path("/lens/github")
        .http_only(true)
        .same_site(cookie::SameSite::Lax)
        .secure(github.public_url.scheme() == "https")
        .max_age(cookie::time::Duration::minutes(10))
        .build()
        .to_string()
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn page(body: String) -> Response {
    let html = format!(
        "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Connect GitHub to Lens</title><style>body{{margin:0;background:#f8fafb;color:#18212b;font:16px/1.5 system-ui,sans-serif;padding:32px 16px}}main{{box-sizing:border-box;max-width:520px;margin:5vh auto;background:white;border:1px solid #e2e6eb;border-radius:16px;padding:32px;box-shadow:0 4px 24px #17233008}}h1{{font-size:24px;line-height:1.25;margin:0 0 20px}}p{{color:#526070}}dl{{margin:24px 0;padding:16px;background:#f7f9fa;border-radius:8px}}dt,label{{font-size:13px;font-weight:600;color:#526070}}dd{{margin:4px 0 16px;overflow-wrap:anywhere}}dd:last-child{{margin-bottom:0}}select,button{{box-sizing:border-box;width:100%;font:inherit;border-radius:8px;padding:12px;margin:8px 0 12px}}select{{background:white;border:1px solid #cdd5dd}}button{{background:#1d2834;color:white;border:0;cursor:pointer;font-weight:600}}button.secondary{{background:#eef2f5;color:#283746}}a{{color:#315a87}}.error{{color:#a33434}}@media(max-width:440px){{main{{padding:24px;margin:16px auto}}}}</style><main>{body}</main></html>"
    );
    ([(header::CACHE_CONTROL,"no-store"),(header::REFERRER_POLICY,"no-referrer"),(header::CONTENT_SECURITY_POLICY,"default-src 'none'; style-src 'unsafe-inline'; form-action 'self' https://github.com; base-uri 'none'; frame-ancestors 'none'")], Html(html)).into_response()
}

fn return_url(handshake: &BrokerHandshake) -> Url {
    let mut url = Url::parse(&handshake.redirect_uri).expect("validated callback");
    url.query_pairs_mut()
        .append_pair("state", &handshake.local_state);
    url
}

pub(super) async fn connect<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, GitHubError> {
    let github = enabled(&app)?;
    let handoff = handshake(&app, &id).await?;
    if handoff.handshake.state != BrokerHandshakeState::Pending {
        return Ok(page(
            "<h1>Connection already confirmed</h1><p>Return to your Lens instance to continue.</p>"
                .into(),
        ));
    }
    let authorization = load(&app.store, &id).await?;
    if !handoff.handshake.browser_claimed {
        let secret = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        app.store.claim_handshake(handoff, hash(&secret)).await?;
        let private = cookie::Cookie::build((BROWSER_COOKIE, secret))
            .path("/lens/github")
            .http_only(true)
            .same_site(cookie::SameSite::Lax)
            .secure(github.public_url.scheme() == "https")
            .max_age(cookie::time::Duration::minutes(10))
            .build();
        let mut headers = HeaderMap::new();
        headers.append(
            header::SET_COOKIE,
            cookie(github, &id).parse().expect("valid cookie"),
        );
        headers.append(
            header::SET_COOKIE,
            private.to_string().parse().expect("valid cookie"),
        );
        headers.insert(
            header::CACHE_CONTROL,
            "no-store".parse().expect("fixed header"),
        );
        headers.insert(
            header::REFERRER_POLICY,
            "no-referrer".parse().expect("fixed header"),
        );
        let install = authorization.authorization.state == AuthorizationState::Installing;
        return Ok((
            headers,
            Redirect::to(&github.authorization_url(&id, install)),
        )
            .into_response());
    }
    browser(&app, &headers, &id).await?;
    if matches!(
        authorization.authorization.state,
        AuthorizationState::Installing | AuthorizationState::Pending
    ) {
        let install = authorization.authorization.state == AuthorizationState::Installing;
        return Ok((
            [
                (header::SET_COOKIE, cookie(github, &id)),
                (header::CACHE_CONTROL, "no-store".into()),
                (header::REFERRER_POLICY, "no-referrer".into()),
            ],
            Redirect::to(&github.authorization_url(&id, install)),
        )
            .into_response());
    }
    let details = format!(
        "<h1>Connect GitHub to Lens</h1><dl><dt>Lens instance</dt><dd>{}</dd><dt>Agent</dt><dd>{}</dd></dl>",
        escape(&handoff.handshake.lens_origin),
        escape(&handoff.handshake.agent)
    );
    let action = format!("/lens/github/service/connect/{id}");
    let hidden = format!(
        "<input type=\"hidden\" name=\"state\" value=\"{}\">",
        escape(&github.state(&id))
    );
    let choose = match authorization.authorization.state {
        AuthorizationState::Ready { repositories } if !repositories.is_empty() => {
            let options = repositories.iter().map(|repository| format!("<option value=\"{}\">{}</option>",repository.id,escape(&repository.full_name))).collect::<String>();
            format!("<form method=\"post\" action=\"{action}\">{hidden}<label for=\"repository\">GitHub repository</label><select id=\"repository\" name=\"repository_id\" required>{options}</select><button type=\"submit\">Connect repository</button></form>")
        }
        AuthorizationState::Ready { .. } => "<p>No repository with write access is available yet. Install Lens or update its repository access, then retry. Your organization may need to approve the installation.</p>".into(),
        AuthorizationState::Failed { error } => format!("<p class=\"error\">{}</p>",escape(&error)),
        _ => "<p>GitHub authorization is still in progress. Retry to continue.</p>".into(),
    };
    let controls = format!(
        "<form method=\"post\" action=\"{action}\">{hidden}<button class=\"secondary\" name=\"action\" value=\"install\">Install Lens or update repository access</button><button class=\"secondary\" name=\"action\" value=\"retry\">Retry GitHub authorization</button></form><p><a href=\"{}\">Return to Lens</a></p>",
        escape(return_url(&handoff.handshake).as_str())
    );
    Ok(page(format!("{details}{choose}{controls}")))
}

#[derive(Deserialize)]
pub(super) struct Consent {
    state: String,
    repository_id: Option<u64>,
    action: Option<String>,
}

pub(super) async fn consent<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(request): Form<Consent>,
) -> Result<Response, GitHubError> {
    let github = enabled(&app)?;
    browser(&app, &headers, &id).await?;
    if cookie_state(&headers).as_deref() != Some(&request.state)
        || headers
            .get(header::ORIGIN)
            .and_then(|value| value.to_str().ok())
            != Some(
                app.settings
                    .public_url
                    .origin()
                    .ascii_serialization()
                    .as_str(),
            )
    {
        return Err(GitHubError::Forbidden);
    }
    let handoff = handshake(&app, &id).await?;
    if handoff.handshake.state != BrokerHandshakeState::Pending {
        return Err(GitHubError::Expired);
    }
    let authorization = load(&app.store, &id).await?;
    if let Some(action) = request.action {
        if !matches!(
            authorization.authorization.state,
            AuthorizationState::Ready { .. } | AuthorizationState::Failed { .. }
        ) {
            return Err(GitHubError::Expired);
        }
        let install = match action.as_str() {
            "install" => true,
            "retry" => false,
            _ => return Err(GitHubError::Invalid("Choose a GitHub connection action")),
        };
        app.store
            .transition(
                authorization,
                if install {
                    AuthorizationState::Installing
                } else {
                    AuthorizationState::Pending
                },
            )
            .await?;
        return Ok((
            [
                (header::CACHE_CONTROL, "no-store"),
                (header::REFERRER_POLICY, "no-referrer"),
            ],
            Redirect::to(&github.authorization_url(&id, install)),
        )
            .into_response());
    }
    let AuthorizationState::Ready { repositories } = &authorization.authorization.state else {
        return Err(GitHubError::Expired);
    };
    let repository = repositories
        .iter()
        .find(|repository| Some(repository.id) == request.repository_id)
        .ok_or(GitHubError::Forbidden)?;
    let connection = Connection {
        agent: handoff.handshake.agent.clone(),
        repository_id: repository.id,
        repository: repository.full_name.clone(),
        installation_id: repository.installation_id,
        default_branch: repository.default_branch.clone(),
        connected_at: Utc::now(),
    };
    let current = github.verify_connection(&connection).await?;
    let connection = Connection {
        repository: current.full_name,
        default_branch: current.default_branch,
        ..connection
    };
    let code = format!(
        "{id}.{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let mut target = return_url(&handoff.handshake);
    target.query_pairs_mut().append_pair("code", &code);
    app.store
        .select_repository(authorization, handoff, hash(&code), &connection)
        .await?;
    Ok((
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        Redirect::to(target.as_str()),
    )
        .into_response())
}

pub(super) async fn redeem<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    Json(request): Json<RedeemRequest>,
) -> Result<Json<Redeemed>, GitHubError> {
    enabled(&app)?;
    if !(43..=128).contains(&request.code_verifier.len()) || request.code.len() > 200 {
        return Err(GitHubError::Forbidden);
    }
    let (id, _) = request.code.split_once('.').ok_or(GitHubError::Forbidden)?;
    let stored = handshake(&app, id).await?;
    let BrokerHandshakeState::Selected {
        code_hash,
        connection,
    } = &stored.handshake.state
    else {
        return Err(GitHubError::Expired);
    };
    if hash(&request.code) != *code_hash
        || URL_SAFE_NO_PAD.encode(Sha256::digest(&request.code_verifier))
            != stored.handshake.code_challenge
    {
        return Err(GitHubError::Forbidden);
    }
    let capability = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let connection = BrokerConnection {
        id: uuid::Uuid::new_v4().to_string(),
        connection: connection.clone(),
        lens_origin: stored.handshake.lens_origin.clone(),
        capability_hash: hash(&capability),
        revoked: false,
    };
    let response = Redeemed {
        connection: connection.connection.clone(),
        lens_origin: connection.lens_origin.clone(),
        credential: RemoteCredentials {
            connection_id: connection.id.clone(),
            capability,
        },
    };
    app.store.redeem_handshake(stored, &connection).await?;
    Ok(Json(response))
}

async fn authenticated<R>(
    app: &App<R>,
    headers: &HeaderMap,
    id: &str,
    allow_revoked: bool,
) -> Result<StoredBrokerConnection, GitHubError> {
    enabled(app)?;
    uuid::Uuid::parse_str(id).map_err(|_| GitHubError::Forbidden)?;
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(GitHubError::Forbidden)?;
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(GitHubError::Forbidden);
    }
    let stored = app
        .store
        .broker_connection(id)
        .await?
        .ok_or(GitHubError::Forbidden)?;
    if hash(token) != stored.connection.capability_hash
        || (stored.connection.revoked && !allow_revoked)
    {
        return Err(GitHubError::Forbidden);
    }
    Ok(stored)
}

pub(super) async fn status<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<ServiceStatus>, GitHubError> {
    let stored = authenticated(&app, &headers, &id, false).await?;
    let github = enabled(&app)?;
    let connection = stored.connection.connection;
    let (connection, available, availability_error) =
        match github.verify_connection(&connection).await {
            Ok(repository) => (
                Connection {
                    repository: repository.full_name,
                    default_branch: repository.default_branch,
                    ..connection
                },
                true,
                String::new(),
            ),
            Err(error) => (connection, false, error.to_string()),
        };
    Ok(Json(ServiceStatus {
        connection,
        app_slug: github.slug.clone(),
        available,
        availability_error,
    }))
}

pub(super) async fn disconnect<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, GitHubError> {
    let stored = authenticated(&app, &headers, &id, true).await?;
    if !stored.connection.revoked {
        app.store.revoke_broker_connection(stored).await?;
    }
    Ok(Json(serde_json::json!({"disconnected":true})))
}

pub(super) async fn report<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(mut report): Json<ReportInput>,
) -> Result<Json<Published>, GitHubError> {
    let stored = authenticated(&app, &headers, &id, false).await?;
    let binding = stored.connection;
    if report.run.agent != binding.connection.agent {
        return Err(GitHubError::Forbidden);
    }
    report.bind_urls(&binding.lens_origin)?;
    let published = super::report::publish_input(
        enabled(&app)?,
        &app.store.0,
        &binding.id,
        &binding.connection,
        report,
    )
    .await?;
    Ok(Json(published))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    fn admission_is_bounded_and_recovers_after_window() {
        let now = Instant::now();
        let admission = Admission(Mutex::new((now, 29)));
        assert!(admission.admit(now).is_ok());
        assert!(matches!(
            admission.admit(now),
            Err(GitHubError::RateLimited)
        ));
        assert!(admission.admit(now + Duration::from_secs(60)).is_ok());
    }
}
