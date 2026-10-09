use std::{sync::Arc, time::Duration};

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{TimeDelta, Utc};
use lens_auth::SessionRepository;
use lens_contract::github::{Authorization, AuthorizationState, Owner};
use litellm_storage_clickhouse::github::remote_credentials_key;
use reqwest::Method;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use url::{Host, Url};

use super::report::{Published, ReportInput};
use super::service::{HandoffRequest, HandoffResponse, RedeemRequest, Redeemed, ServiceStatus};
use super::{App, ConnectionStatus, GitHubError, Status, cookie_named, load, return_to_agent};

pub(super) const CALLBACK_PATH: &str = "/lens/github/service/callback";
const REMOTE_COOKIE: &str = "lens_github_connection";

fn cookie_state(headers: &HeaderMap) -> Option<String> {
    cookie_named(headers, REMOTE_COOKIE)
}

fn return_to_lens<R>(app: &App<R>, authorization: &Authorization) -> Response {
    let mut response = return_to_agent(&app.settings.public_url, authorization);
    let cookie = cookie::Cookie::build((REMOTE_COOKIE, ""))
        .path(CALLBACK_PATH)
        .http_only(true)
        .same_site(cookie::SameSite::Lax)
        .secure(app.settings.public_url.scheme() == "https")
        .max_age(cookie::time::Duration::ZERO)
        .build();
    response.headers_mut().append(
        header::SET_COOKIE,
        cookie.to_string().parse().expect("valid cookie"),
    );
    response
}

pub(super) fn secure_origin(url: &Url) -> bool {
    let localhost = match url.host() {
        Some(Host::Domain(domain)) => domain == "localhost",
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    };
    (url.scheme() == "https" || (url.scheme() == "http" && localhost))
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RemoteCredentials {
    pub connection_id: String,
    pub capability: String,
}

pub(super) struct Remote {
    base: Url,
    http: reqwest::Client,
}

impl Remote {
    pub(super) fn new(base: Url) -> Result<Self, GitHubError> {
        if !secure_origin(&base) || base.path() != "/" {
            return Err(GitHubError::Configuration(
                "LENS_GITHUB_SERVICE_URL must be an HTTPS origin (HTTP localhost is supported for development)",
            ));
        }
        let http = reqwest::Client::builder()
            .user_agent("lens-github-connector")
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(40))
            .build()
            .map_err(GitHubError::Transport)?;
        Ok(Self { base, http })
    }

    pub(super) fn origin(&self) -> String {
        self.base.origin().ascii_serialization()
    }

    async fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        credential: Option<&RemoteCredentials>,
        body: Option<serde_json::Value>,
    ) -> Result<T, GitHubError> {
        let url = self
            .base
            .join(path)
            .map_err(|_| GitHubError::Invalid("Invalid Lens GitHub service path"))?;
        if url.origin() != self.base.origin() {
            return Err(GitHubError::Forbidden);
        }
        let request = self.http.request(method, url);
        let request = match credential {
            Some(credential) => request.bearer_auth(&credential.capability),
            None => request,
        };
        let request = match body {
            Some(body) => request.json(&body),
            None => request,
        };
        let response = request.send().await.map_err(GitHubError::Transport)?;
        if !response.status().is_success() {
            return Err(GitHubError::Upstream {
                status: response.status().as_u16(),
            });
        }
        response.json().await.map_err(GitHubError::Decode)
    }

    async fn start(&self, request: HandoffRequest) -> Result<HandoffResponse, GitHubError> {
        let response: HandoffResponse = self
            .request(
                Method::POST,
                "/lens/github/service/authorize",
                None,
                Some(serde_json::to_value(request).map_err(|_| GitHubError::Credentials)?),
            )
            .await?;
        let url = Url::parse(&response.authorization_url).map_err(|_| GitHubError::Forbidden)?;
        if url.origin() != self.base.origin()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(GitHubError::Forbidden);
        }
        Ok(response)
    }

    async fn redeem(&self, code: &str, verifier: &str) -> Result<Redeemed, GitHubError> {
        self.request(
            Method::POST,
            "/lens/github/service/redeem",
            None,
            Some(
                serde_json::to_value(RedeemRequest {
                    code: code.into(),
                    code_verifier: verifier.into(),
                })
                .map_err(|_| GitHubError::Credentials)?,
            ),
        )
        .await
    }

    pub(super) async fn status(
        &self,
        credentials: &RemoteCredentials,
    ) -> Result<ServiceStatus, GitHubError> {
        self.request(
            Method::GET,
            &format!(
                "/lens/github/service/connections/{}",
                credentials.connection_id
            ),
            Some(credentials),
            None,
        )
        .await
    }

    async fn revoke(&self, credentials: &RemoteCredentials) -> Result<(), GitHubError> {
        let _: serde_json::Value = self
            .request(
                Method::DELETE,
                &format!(
                    "/lens/github/service/connections/{}",
                    credentials.connection_id
                ),
                Some(credentials),
                None,
            )
            .await?;
        Ok(())
    }

    pub(super) async fn report(
        &self,
        credentials: &RemoteCredentials,
        report: ReportInput,
    ) -> Result<Published, GitHubError> {
        self.request(
            Method::POST,
            &format!(
                "/lens/github/service/connections/{}/report",
                credentials.connection_id
            ),
            Some(credentials),
            Some(serde_json::to_value(report).map_err(|_| GitHubError::Credentials)?),
        )
        .await
    }

    pub(super) async fn progress(
        &self,
        credentials: &RemoteCredentials,
        input: super::progress::ProgressInput,
    ) -> Result<lens_contract::github::ProgressPublished, GitHubError> {
        self.request(
            Method::POST,
            &format!(
                "/lens/github/service/connections/{}/progress",
                credentials.connection_id
            ),
            Some(credentials),
            Some(serde_json::to_value(input).map_err(|_| GitHubError::Credentials)?),
        )
        .await
    }
}

pub(super) async fn credentials<R>(
    app: &App<R>,
    scope: &str,
    agent: &str,
) -> Result<Option<RemoteCredentials>, GitHubError> {
    app.store
        .remote_credentials(scope, agent)
        .await?
        .map(|value| {
            let credentials: RemoteCredentials = app
                .settings
                .cipher
                .decrypt(&value, &remote_credentials_key(scope, agent))?;
            uuid::Uuid::parse_str(&credentials.connection_id)
                .map_err(|_| GitHubError::Credentials)?;
            if credentials.capability.len() != 64
                || !credentials
                    .capability
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(GitHubError::Credentials);
            }
            Ok(credentials)
        })
        .transpose()
}

pub(super) async fn status<R>(
    app: &App<R>,
    remote: &Remote,
    scope: &str,
    agent: &str,
) -> Result<Json<Status>, GitHubError> {
    let cleanup = cleanup_retired(app, remote, scope, agent).await;
    if let Err(GitHubError::Storage(error)) = cleanup {
        return Err(error.into());
    }
    let connection = app.store.connection(scope, agent).await?;
    let (connection, app_slug) = match connection {
        Some(connection) => {
            let result = match cleanup {
                Err(error) => Err(error),
                Ok(()) => match credentials(app, scope, agent).await {
                    Ok(Some(credentials)) => remote.status(&credentials).await,
                    Ok(None) | Err(GitHubError::Credentials) => Err(GitHubError::Credentials),
                    Err(error) => return Err(error),
                },
            };
            match result {
                Ok(status) => {
                    if status.connection.agent != agent
                        || status.connection.repository_id != connection.repository_id
                        || status.connection.installation_id != connection.installation_id
                    {
                        return Err(GitHubError::Credentials);
                    }
                    (
                        Some(ConnectionStatus {
                            connection: status.connection,
                            available: status.available,
                            availability_error: status.availability_error,
                        }),
                        Some(status.app_slug),
                    )
                }
                Err(error) => (
                    Some(ConnectionStatus {
                        connection,
                        available: false,
                        availability_error: error.to_string(),
                    }),
                    None,
                ),
            }
        }
        None => (None, None),
    };
    Ok(Json(Status {
        configured: true,
        service_origin: Some(remote.origin()),
        app_slug,
        connection,
    }))
}

pub(super) async fn authorize<R>(
    app: &App<R>,
    remote: &Remote,
    owner: Owner,
    agent: String,
    install: bool,
) -> Result<Response, GitHubError> {
    let authorization = Authorization {
        id: uuid::Uuid::new_v4().to_string(),
        owner,
        agent: agent.clone(),
        expires_at: Utc::now() + TimeDelta::minutes(10),
        state: AuthorizationState::Pending,
    };
    let state = app.settings.cipher.state(&authorization.id);
    let verifier = app.settings.cipher.verifier(&authorization.id);
    app.store.create_authorization(&authorization).await?;
    let response = remote
        .start(HandoffRequest {
            redirect_uri: app
                .settings
                .public_url
                .join(CALLBACK_PATH)
                .expect("fixed path")
                .into(),
            state: state.clone(),
            code_challenge: URL_SAFE_NO_PAD.encode(Sha256::digest(verifier)),
            agent,
            install,
        })
        .await?;
    let cookie = cookie::Cookie::build((REMOTE_COOKIE, state))
        .path(CALLBACK_PATH)
        .http_only(true)
        .same_site(cookie::SameSite::Lax)
        .secure(app.settings.public_url.scheme() == "https")
        .max_age(cookie::time::Duration::minutes(10))
        .build();
    Ok(([(header::SET_COOKIE,cookie.to_string()),(header::CACHE_CONTROL,"no-store".into()),(header::REFERRER_POLICY,"no-referrer".into())], Json(serde_json::json!({
        "authorization_url":response.authorization_url,"authorization_id":authorization.id,"expires_at":authorization.expires_at,
    }))).into_response())
}

#[derive(Deserialize)]
pub(super) struct Callback {
    #[serde(default)]
    state: String,
    code: Option<String>,
}

pub(super) async fn callback<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
    Query(query): Query<Callback>,
) -> Response {
    let result = async {
        let id = app.settings.cipher.authorization_id(&query.state)?;
        if cookie_state(&headers).as_deref() != Some(&query.state) {
            return Err(GitHubError::Forbidden);
        }
        let stored = load(&app.store, &id).await?;
        if stored.authorization.state != AuthorizationState::Pending {
            return Err(GitHubError::Expired);
        }
        let authorization = stored.authorization.clone();
        app.store
            .transition(stored, AuthorizationState::Exchanging)
            .await?;
        let connected = complete(&app, &authorization, query.code.as_deref()).await;
        if let Err(error) = connected {
            let stored = load(&app.store, &id).await?;
            app.store
                .transition(
                    stored,
                    AuthorizationState::Failed {
                        error: error.to_string(),
                    },
                )
                .await?;
        }
        Ok::<_, GitHubError>(return_to_lens(&app, &authorization))
    }
    .await;
    match result {
        Ok(response) => response,
        Err(_) => {
            if cookie_state(&headers).as_deref() == Some(&query.state)
                && let Ok(id) = app.settings.cipher.authorization_id(&query.state)
                && let Ok(Some(stored)) = app.store.authorization(&id).await
            {
                return return_to_lens(&app, &stored.authorization);
            }
            super::callback_failure(&app, &headers, &query.state).await
        }
    }
}

async fn complete<R>(
    app: &App<R>,
    authorization: &Authorization,
    code: Option<&str>,
) -> Result<(), GitHubError> {
    let remote = app
        .settings
        .remote
        .as_ref()
        .ok_or(GitHubError::NotConfigured)?;
    let code = code
        .filter(|value| !value.is_empty() && value.len() <= 200)
        .ok_or(GitHubError::Invalid(
            "GitHub connection was cancelled. Connect again when ready",
        ))?;
    let redeemed = remote
        .redeem(code, &app.settings.cipher.verifier(&authorization.id))
        .await?;
    let record = remote_credentials_key(&authorization.owner.scope, &authorization.agent);
    let saved = async {
        if redeemed.connection.agent != authorization.agent
            || redeemed.lens_origin != app.settings.public_url.origin().ascii_serialization()
        {
            return Err(GitHubError::Forbidden);
        }
        uuid::Uuid::parse_str(&redeemed.credential.connection_id)
            .map_err(|_| GitHubError::Credentials)?;
        if redeemed.credential.capability.len() != 64
            || !redeemed
                .credential
                .capability
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(GitHubError::Credentials);
        }
        let encrypted = app.settings.cipher.encrypt(&redeemed.credential, &record)?;
        let stored = load(&app.store, &authorization.id).await?;
        app.store
            .connect_remote(stored, &redeemed.connection, encrypted)
            .await
            .map_err(GitHubError::from)
    }
    .await;
    match saved {
        Ok(_) => {
            let _ = cleanup_retired(
                app,
                remote,
                &authorization.owner.scope,
                &authorization.agent,
            )
            .await;
            Ok(())
        }
        Err(error) => {
            let _ = remote.revoke(&redeemed.credential).await;
            Err(error)
        }
    }
}

pub(super) async fn disconnect<R>(
    app: &App<R>,
    remote: &Remote,
    scope: &str,
    agent: &str,
) -> Result<StatusCode, GitHubError> {
    app.store.disconnect_remote(scope, agent).await?;
    match cleanup_retired(app, remote, scope, agent).await {
        Ok(()) | Err(GitHubError::Credentials) => {}
        Err(error) => return Err(error),
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn cleanup_retired<R>(
    app: &App<R>,
    remote: &Remote,
    scope: &str,
    agent: &str,
) -> Result<(), GitHubError> {
    let record = remote_credentials_key(scope, agent);
    for encrypted in app.store.retired_remote_credentials(scope, agent).await? {
        let credentials: RemoteCredentials = app.settings.cipher.decrypt(&encrypted, &record)?;
        remote.revoke(&credentials).await?;
        app.store
            .remove_retired_remote_credentials(scope, agent, &encrypted)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::https("https://service.example", true)]
    #[case::localhost("http://localhost:3105", true)]
    #[case::loopback("http://127.0.0.1:3105", true)]
    #[case::remote_http("http://service.example", false)]
    #[case::credentials("https://user@service.example", false)]
    #[case::path("https://service.example/path", false)]
    #[case::query("https://service.example?other=true", false)]
    #[case::fragment("https://service.example#token", false)]
    fn service_origin_is_explicit_and_secure(#[case] url: &str, #[case] accepted: bool) {
        assert_eq!(Remote::new(url.parse().unwrap()).is_ok(), accepted);
    }
}
