mod client;
mod credentials;
mod remote;
mod report;
mod service;

use std::{sync::Arc, time::Duration};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, Method, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post, put},
};
use chrono::{TimeDelta, Utc};
use lens_auth::{Authentication, SessionRepository};
use lens_contract::{
    auth::{Identity, Role},
    github::{Authorization, AuthorizationState, Connection, Owner},
};
use litellm_storage_clickhouse::github::{GitHubStore, StoredAuthorization};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

pub use crate::error::GitHubError;
use crate::{auth, routing::PublicRoutes};
pub use client::GitHubApp;
use credentials::CredentialCipher;
use remote::Remote;

const STATE_COOKIE: &str = "lens_github_state";

pub struct GitHubSettings {
    github: Option<GitHubApp>,
    remote: Option<Remote>,
    public_url: Url,
    cipher: CredentialCipher,
    service_enabled: bool,
}

impl GitHubSettings {
    pub fn new(
        github: Option<GitHubApp>,
        public_url: Url,
        admin_token: &str,
        service_url: Option<Url>,
        service_enabled: bool,
    ) -> Result<Self, GitHubError> {
        if (github.is_some() && service_url.is_some()) || (service_enabled && github.is_none()) {
            return Err(GitHubError::Configuration(
                "Use official App credentials on the service, or LENS_GITHUB_SERVICE_URL on a self-hosted instance",
            ));
        }
        let remote = service_url.map(Remote::new).transpose()?;
        if (remote.is_some() || service_enabled) && !remote::secure_origin(&public_url) {
            return Err(GitHubError::Configuration(
                "LENS_PUBLIC_URL must use HTTPS (HTTP localhost is supported for development)",
            ));
        }
        Ok(Self {
            github,
            remote,
            public_url,
            cipher: CredentialCipher::new(admin_token)?,
            service_enabled,
        })
    }
}

struct App<R> {
    authentication: Arc<Authentication<R>>,
    store: GitHubStore,
    settings: GitHubSettings,
    admission: service::Admission,
}

pub fn router<R: SessionRepository + 'static>(
    authentication: Arc<Authentication<R>>,
    store: GitHubStore,
    settings: GitHubSettings,
) -> Router {
    Router::new()
        .public_route("/lens/github/status", get(status::<R>))
        .public_route("/lens/github/authorize", post(authorize::<R>))
        .public_route("/lens/github/authorizations/{id}", get(authorization::<R>))
        .public_route(
            "/lens/github/connections/{agent}",
            put(connect::<R>).delete(disconnect::<R>),
        )
        .public_route("/lens/github/callback", get(callback::<R>))
        .public_route("/lens/github/setup", get(setup::<R>))
        .public_route("/lens/github/report", post(report::publish::<R>))
        .public_route("/lens/github/service/authorize", post(service::start::<R>))
        .public_route(
            "/lens/github/service/connect/{id}",
            get(service::connect::<R>).post(service::consent::<R>),
        )
        .public_route("/lens/github/service/redeem", post(service::redeem::<R>))
        .public_route(
            "/lens/github/service/connections/{id}",
            get(service::status::<R>).delete(service::disconnect::<R>),
        )
        .public_route(
            "/lens/github/service/connections/{id}/report",
            post(service::report::<R>),
        )
        .public_route(remote::CALLBACK_PATH, get(remote::callback::<R>))
        .layer(axum::middleware::map_response(
            |mut response: Response| async move {
                response.headers_mut().insert(
                    header::CACHE_CONTROL,
                    "no-store".parse().expect("fixed header"),
                );
                response.headers_mut().insert(
                    header::REFERRER_POLICY,
                    "no-referrer".parse().expect("fixed header"),
                );
                response
            },
        ))
        .with_state(Arc::new(App {
            authentication,
            store,
            settings,
            admission: service::Admission::default(),
        }))
}

fn owner(identity: &Identity, write: bool) -> Result<Owner, GitHubError> {
    if write
        && matches!(
            identity.user_role,
            Role::ProxyAdminViewer | Role::InternalUserViewer | Role::Customer
        )
    {
        return Err(GitHubError::Forbidden);
    }
    let scope = match identity.team_id.as_ref().filter(|team| !team.is_empty()) {
        Some(team) => team.clone(),
        None if matches!(
            identity.user_role,
            Role::ProxyAdmin | Role::ProxyAdminViewer
        ) =>
        {
            String::new()
        }
        None => return Err(GitHubError::Forbidden),
    };
    let subject = identity
        .user_id
        .as_ref()
        .filter(|user| !user.is_empty())
        .cloned()
        .or_else(|| {
            identity
                .token
                .as_ref()
                .filter(|token| !token.is_empty())
                .map(|token| format!("token:{:x}", Sha256::digest(token)))
        })
        .ok_or(GitHubError::Forbidden)?;
    Ok(Owner { scope, subject })
}

fn agent(value: &str) -> Result<(), GitHubError> {
    if value.trim().is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(GitHubError::Invalid(
            "Choose an agent before connecting GitHub",
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
struct AgentQuery {
    agent: String,
}

#[derive(Serialize)]
struct ConnectionStatus {
    #[serde(flatten)]
    connection: Connection,
    available: bool,
    availability_error: String,
}

#[derive(Serialize)]
struct Status {
    configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_origin: Option<String>,
    app_slug: Option<String>,
    connection: Option<ConnectionStatus>,
}

async fn status<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
    Query(query): Query<AgentQuery>,
) -> Result<Json<Status>, GitHubError> {
    let identity = auth::identity(&app.authentication, &headers, &Method::GET).await?;
    let owner = owner(&identity, false)?;
    agent(&query.agent)?;
    if let Some(remote) = &app.settings.remote {
        return remote::status(&app, remote, &owner.scope, &query.agent).await;
    }
    let connection = app.store.connection(&owner.scope, &query.agent).await?;
    let connection = match connection {
        Some(connection) => {
            let verification = match &app.settings.github {
                Some(github) => github.verify_connection(&connection).await,
                None => Err(GitHubError::NotConfigured),
            };
            let (connection, available, availability_error) = match verification {
                Ok(repo) => (
                    Connection {
                        repository: repo.full_name,
                        default_branch: repo.default_branch,
                        ..connection
                    },
                    true,
                    String::new(),
                ),
                Err(error) => (connection, false, error.to_string()),
            };
            Some(ConnectionStatus {
                connection,
                available,
                availability_error,
            })
        }
        None => None,
    };
    Ok(Json(Status {
        configured: app.settings.github.is_some(),
        service_origin: None,
        app_slug: app
            .settings
            .github
            .as_ref()
            .map(|github| github.slug.clone()),
        connection,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Authorize {
    agent: String,
    #[serde(default)]
    install: bool,
}

async fn authorize<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
    Json(request): Json<Authorize>,
) -> Result<Response, GitHubError> {
    let identity = auth::identity(&app.authentication, &headers, &Method::POST).await?;
    let owner = owner(&identity, true)?;
    agent(&request.agent)?;
    if let Some(remote) = &app.settings.remote {
        return remote::authorize(&app, remote, owner, request.agent, request.install).await;
    }
    let github = app
        .settings
        .github
        .as_ref()
        .ok_or(GitHubError::NotConfigured)?;
    let authorization = Authorization {
        id: uuid::Uuid::new_v4().to_string(),
        owner,
        agent: request.agent,
        expires_at: Utc::now() + TimeDelta::minutes(10),
        state: if request.install {
            AuthorizationState::Installing
        } else {
            AuthorizationState::Pending
        },
    };
    app.store.create_authorization(&authorization).await?;
    let cookie = cookie::Cookie::build((STATE_COOKIE, github.state(&authorization.id)))
        .path("/lens/github")
        .http_only(true)
        .same_site(cookie::SameSite::Lax)
        .secure(github.public_url.scheme() == "https")
        .max_age(cookie::time::Duration::minutes(10))
        .build();
    Ok((
        [
            (header::SET_COOKIE, cookie.to_string()),
            (header::CACHE_CONTROL, "no-store".into()),
            (header::REFERRER_POLICY, "no-referrer".into()),
        ],
        Json(serde_json::json!({
            "authorization_url":github.authorization_url(&authorization.id, request.install),
            "authorization_id":authorization.id, "expires_at":authorization.expires_at,
        })),
    )
        .into_response())
}

async fn load(store: &GitHubStore, id: &str) -> Result<StoredAuthorization, GitHubError> {
    uuid::Uuid::parse_str(id).map_err(|_| GitHubError::Expired)?;
    let stored = store.authorization(id).await?.ok_or(GitHubError::Expired)?;
    if stored.authorization.expires_at <= Utc::now() {
        return Err(GitHubError::Expired);
    }
    Ok(stored)
}

async fn authorization<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, GitHubError> {
    let identity = auth::identity(&app.authentication, &headers, &Method::GET).await?;
    let owner = owner(&identity, true)?;
    let stored = load(&app.store, &id).await?;
    if stored.authorization.owner != owner {
        return Err(GitHubError::Forbidden);
    }
    let state = match stored.authorization.state {
        AuthorizationState::Installing
        | AuthorizationState::Exchanging
        | AuthorizationState::Pending => serde_json::json!({"status":"pending","repositories":[]}),
        AuthorizationState::Ready { repositories } => {
            serde_json::json!({"status":"ready","repositories":repositories})
        }
        AuthorizationState::Failed { error } => {
            serde_json::json!({"status":"failed","repositories":[],"error":error})
        }
        AuthorizationState::Connected => {
            serde_json::json!({"status":"connected","repositories":[]})
        }
    };
    Ok(Json(state))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Connect {
    authorization_id: String,
    repository_id: u64,
}

async fn connect<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
    Path(agent): Path<String>,
    Json(request): Json<Connect>,
) -> Result<Json<Connection>, GitHubError> {
    let identity = auth::identity(&app.authentication, &headers, &Method::PUT).await?;
    let owner = owner(&identity, true)?;
    let github = app
        .settings
        .github
        .as_ref()
        .ok_or(GitHubError::NotConfigured)?;
    let stored = load(&app.store, &request.authorization_id).await?;
    if stored.authorization.owner != owner || stored.authorization.agent != agent {
        return Err(GitHubError::Forbidden);
    }
    let AuthorizationState::Ready { repositories } = &stored.authorization.state else {
        return Err(GitHubError::Expired);
    };
    let repo = repositories
        .iter()
        .find(|repo| repo.id == request.repository_id)
        .ok_or(GitHubError::Forbidden)?;
    let connection = Connection {
        agent,
        repository_id: repo.id,
        repository: repo.full_name.clone(),
        installation_id: repo.installation_id,
        default_branch: repo.default_branch.clone(),
        connected_at: Utc::now(),
    };
    let current = github.verify_connection(&connection).await?;
    let connection = Connection {
        repository: current.full_name,
        default_branch: current.default_branch,
        ..connection
    };
    app.store.connect(stored, &connection).await?;
    Ok(Json(connection))
}

async fn disconnect<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
    Path(agent): Path<String>,
) -> Result<StatusCode, GitHubError> {
    let identity = auth::identity(&app.authentication, &headers, &Method::DELETE).await?;
    let owner = owner(&identity, true)?;
    if let Some(remote) = &app.settings.remote {
        return remote::disconnect(&app, remote, &owner.scope, &agent).await;
    }
    app.store.disconnect(&owner.scope, &agent).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct Callback {
    #[serde(default)]
    state: String,
    code: Option<String>,
    error: Option<String>,
}

fn cookie_state(headers: &HeaderMap) -> Option<String> {
    cookie_named(headers, STATE_COOKIE)
}

fn cookie_named(headers: &HeaderMap, name: &str) -> Option<String> {
    cookie::Cookie::split_parse(headers.get(header::COOKIE)?.to_str().ok()?)
        .filter_map(Result::ok)
        .find(|cookie| cookie.name() == name)
        .map(|cookie| cookie.value().to_owned())
}

async fn callback_state<R>(
    app: &App<R>,
    headers: &HeaderMap,
    state: &str,
) -> Result<StoredAuthorization, GitHubError> {
    let github = app
        .settings
        .github
        .as_ref()
        .ok_or(GitHubError::NotConfigured)?;
    let id = github.authorization_id(state)?;
    if cookie_state(headers).as_deref() != Some(state) {
        return Err(GitHubError::Forbidden);
    }
    if app.settings.service_enabled && app.store.handshake(&id).await?.is_some() {
        service::browser(app, headers, &id).await?;
    }
    load(&app.store, &id).await
}

fn return_to_agent(public_url: &Url, authorization: &Authorization) -> Response {
    let mut url = public_url.join("/ui/").expect("fixed path");
    url.query_pairs_mut()
        .append_pair("tab", "agents")
        .append_pair("github_agent", &authorization.agent)
        .append_pair("github_authorization", &authorization.id);
    let cookie = cookie::Cookie::build((STATE_COOKIE, ""))
        .path("/lens/github")
        .http_only(true)
        .same_site(cookie::SameSite::Lax)
        .secure(public_url.scheme() == "https")
        .max_age(cookie::time::Duration::ZERO)
        .build();
    (
        [
            (header::SET_COOKIE, cookie.to_string()),
            (header::CACHE_CONTROL, "no-store".into()),
            (header::REFERRER_POLICY, "no-referrer".into()),
        ],
        Redirect::to(url.as_str()),
    )
        .into_response()
}

async fn callback<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
    Query(query): Query<Callback>,
) -> Response {
    match callback_inner(&app, &headers, &query).await {
        Ok(response) => response,
        Err(_) => callback_failure(&app, &headers, &query.state).await,
    }
}

async fn callback_failure<R>(app: &App<R>, headers: &HeaderMap, state: &str) -> Response {
    if let Some(github) = &app.settings.github
        && cookie_state(headers).as_deref() == Some(state)
        && let Ok(id) = github.authorization_id(state)
        && let Ok(Some(stored)) = app.store.authorization(&id).await
    {
        return return_to_agent(&github.public_url, &stored.authorization);
    }
    (
        StatusCode::BAD_REQUEST,
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        Html("<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>GitHub connection</title><p>This GitHub authorization could not be verified.</p><p><a href=\"/ui/\">Return to Lens</a> and connect GitHub again.</p></html>"),
    ).into_response()
}

async fn callback_inner<R>(
    app: &App<R>,
    headers: &HeaderMap,
    query: &Callback,
) -> Result<Response, GitHubError> {
    let github = app
        .settings
        .github
        .as_ref()
        .ok_or(GitHubError::NotConfigured)?;
    let stored = callback_state(app, headers, &query.state).await?;
    if stored.authorization.state != AuthorizationState::Pending {
        return Err(GitHubError::Expired);
    }
    let authorization = stored.authorization.clone();
    app.store
        .transition(stored, AuthorizationState::Exchanging)
        .await?;
    let result = if query.error.is_some() {
        Err(GitHubError::Invalid(
            "GitHub authorization was cancelled. Connect again when ready",
        ))
    } else if let Some(code) = query
        .code
        .as_ref()
        .filter(|code| !code.is_empty() && code.len() <= 2048)
    {
        tokio::time::timeout(
            Duration::from_secs(90),
            github.repositories(&authorization.id, code),
        )
        .await
        .unwrap_or(Err(GitHubError::Invalid(
            "GitHub authorization timed out. Connect again",
        )))
    } else {
        Err(GitHubError::Invalid(
            "GitHub did not return an authorization code",
        ))
    };
    let state = match result {
        Ok(repositories) => AuthorizationState::Ready { repositories },
        Err(error) => AuthorizationState::Failed {
            error: error.to_string(),
        },
    };
    let stored = load(&app.store, &authorization.id).await?;
    app.store.transition(stored, state).await?;
    if app.settings.service_enabled && app.store.handshake(&authorization.id).await?.is_some() {
        return Ok(service::redirect_to_consent(
            &app.settings.public_url,
            &authorization.id,
        ));
    }
    Ok(return_to_agent(&github.public_url, &authorization))
}

#[derive(Deserialize)]
struct Setup {
    #[serde(default)]
    state: String,
}

async fn setup<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
    Query(query): Query<Setup>,
) -> Response {
    match setup_inner(&app, &headers, &query).await {
        Ok(response) => response,
        Err(_) => callback_failure(&app, &headers, &query.state).await,
    }
}

async fn setup_inner<R>(
    app: &App<R>,
    headers: &HeaderMap,
    query: &Setup,
) -> Result<Response, GitHubError> {
    let github = app
        .settings
        .github
        .as_ref()
        .ok_or(GitHubError::NotConfigured)?;
    let stored = callback_state(app, headers, &query.state).await?;
    if stored.authorization.state != AuthorizationState::Installing {
        return Err(GitHubError::Expired);
    }
    let url = github.authorization_url(&stored.authorization.id, false);
    app.store
        .transition(stored, AuthorizationState::Pending)
        .await?;
    Ok((
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        Redirect::to(&url),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use jsonwebtoken::{EncodingKey, Header, encode};
    use lens_auth::Settings;
    use litellm_http::Client;
    use litellm_storage_clickhouse::{
        Connection as DatabaseConnection, execute_statement, sessions::Sessions,
        state::ClickHouseState,
    };
    use rstest::{fixture, rstest};
    use serde_json::{Value, json};
    use testcontainers_modules::{
        clickhouse::ClickHouse,
        testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner},
    };
    use tower::ServiceExt;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_partial_json, header as match_header, method, path},
    };

    const SECRET: &str = "github-test-gateway-signing-secret-32-characters";
    const ADMIN: &str = "github-test-admin-token-32-characters";

    struct Broker {
        app: Router,
        local: Router,
        origin: Url,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for Broker {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn broker(fixture: &Fixture) -> Broker {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin: Url = format!("http://{}", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let mut github = github(&fixture.provider);
        github.public_url = origin.clone();
        let app = super::router(
            authentication(fixture.store.0.clone()),
            fixture.store.clone(),
            GitHubSettings::new(Some(github), origin.clone(), ADMIN, None, true).unwrap(),
        );
        let server = app.clone();
        let task = tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
        let local = super::router(
            authentication(fixture.store.0.clone()),
            fixture.store.clone(),
            GitHubSettings::new(
                None,
                "https://customer.example".parse().unwrap(),
                ADMIN,
                Some(origin.clone()),
                false,
            )
            .unwrap(),
        );
        Broker {
            app,
            local,
            origin,
            task,
        }
    }

    fn cookies(response: &Response) -> String {
        response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|cookie| cookie.to_str().unwrap().split(';').next().unwrap())
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn form(uri: &str, origin: &Url, cookie: &str, state: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::ORIGIN, origin.origin().ascii_serialization())
            .header(header::COOKIE, cookie)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(
                url::form_urlencoded::Serializer::new(String::new())
                    .append_pair("state", state)
                    .append_pair("repository_id", "10")
                    .finish(),
            ))
            .unwrap()
    }

    async fn local_start(broker: &Broker) -> (Value, String) {
        let response = broker
            .local
            .clone()
            .oneshot(request(
                "POST",
                "/lens/github/authorize",
                Some(&token("team-a", "alice", "team")),
                None,
                Some(json!({"agent":"agent","install":true})),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let cookie = cookies(&response);
        (body(response).await, cookie)
    }

    async fn completed_run(fixture: &Fixture) -> lens_contract::eval::EvalRun {
        use litellm_storage_clickhouse::evals::{EvalStore, RunCompletion, StoredCase};
        let store = EvalStore::new(fixture.store.0.clone());
        let created = store
            .create(
                "team-a",
                serde_json::from_value(json!({
                        "eval":"demo", "agent":"agent", "dataset_id":"test", "revision":1,
                        "version":"a".repeat(40), "branch":"topic", "pr":7,
                        "ci_url":"https://github.com/org/renamed/actions/runs/50", "trials":1,
                "scorers":[{"kind":"task_completed"}]
                    }))
                .unwrap(),
                vec![StoredCase {
                    id: "case".into(),
                    title: "Case".into(),
                    critical: false,
                    input: String::new(),
                    followups: Vec::new(),
                    expected: String::new(),
                }],
                None,
                "https://untrusted-payload.example",
                Utc::now(),
            )
            .await
            .unwrap();
        let finished = store
            .finish("team-a", &created.run.id, Utc::now())
            .await
            .unwrap();
        let lease = store
            .claim_scoring("team-a", &created.run.id, Utc::now())
            .await
            .unwrap()
            .unwrap();
        let template: lens_contract::eval::EvalRun = serde_json::from_str(include_str!(
            "../../../../sdk/tests/fixtures/lens_eval/eval_run_no_baseline.json"
        ))
        .unwrap();
        let mut summary = template.summary.unwrap();
        summary.total = 1;
        summary.passed = 0;
        summary.pass_rate = 0.0;
        store
            .complete(
                "team-a",
                &created.run.id,
                &lease,
                RunCompletion {
                    summary,
                    trials: finished.trials,
                    verdicts: std::collections::BTreeMap::from([("case".into(), false)]),
                },
                Utc::now(),
            )
            .await
            .unwrap()
            .run
    }

    async fn reporting_provider(provider: &MockServer) {
        Mock::given(method("GET")).and(path("/repos/org/renamed/actions/runs/50"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":50,"event":"pull_request","head_sha":"a".repeat(40),"repository":{"id":10},"pull_requests":[{"number":7}]}))).expect(1).mount(provider).await;
        Mock::given(method("GET")).and(path("/repos/org/renamed/pulls/7"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"number":7,"head":{"sha":"a".repeat(40),"repo":{"id":10}},"base":{"sha":"b".repeat(40),"repo":{"id":10}},"merge_commit_sha":"c".repeat(40)}))).expect(1).mount(provider).await;
        Mock::given(method("GET"))
            .and(path("/repos/org/renamed/issues/7/comments"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .expect(1)
            .mount(provider)
            .await;
        Mock::given(method("GET"))
            .and(path(format!(
                "/repos/org/renamed/commits/{}/check-runs",
                "a".repeat(40)
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"check_runs":[]})))
            .expect(1)
            .mount(provider)
            .await;
        Mock::given(method("POST"))
            .and(path("/repos/org/renamed/issues/7/comments"))
            .and(|request: &wiremock::Request| {
                let text = String::from_utf8_lossy(&request.body);
                text.contains("https://customer.example/ui/?tab=evals")
                    && !text.contains("untrusted-payload.example")
            })
            .respond_with(ResponseTemplate::new(201).set_body_json(
                json!({"html_url":"https://github.com/org/renamed/pull/7#issuecomment-1"}),
            ))
            .expect(1)
            .mount(provider)
            .await;
        Mock::given(method("POST"))
            .and(path("/repos/org/renamed/check-runs"))
            .and(|request: &wiremock::Request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                body["details_url"]
                    .as_str()
                    .unwrap()
                    .starts_with("https://customer.example/ui/?tab=evals&eval=demo&eval_run=")
            })
            .respond_with(
                ResponseTemplate::new(201)
                    .set_body_json(json!({"html_url":"https://github.com/org/renamed/checks/1"})),
            )
            .expect(1)
            .mount(provider)
            .await;
    }

    async fn save_remote(fixture: &Fixture, credential: &remote::RemoteCredentials) -> Connection {
        let auth = Authorization {
            id: uuid::Uuid::new_v4().to_string(),
            owner: Owner {
                scope: "team-a".into(),
                subject: "alice".into(),
            },
            agent: "agent".into(),
            expires_at: Utc::now() + TimeDelta::minutes(10),
            state: AuthorizationState::Exchanging,
        };
        fixture.store.create_authorization(&auth).await.unwrap();
        let stored = fixture
            .store
            .authorization(&auth.id)
            .await
            .unwrap()
            .unwrap();
        let connection = Connection {
            agent: "agent".into(),
            repository_id: 10,
            repository: "org/repo".into(),
            installation_id: 42,
            default_branch: "main".into(),
            connected_at: Utc::now(),
        };
        let encrypted = CredentialCipher::new(ADMIN)
            .unwrap()
            .encrypt(
                credential,
                &litellm_storage_clickhouse::github::remote_credentials_key("team-a", "agent"),
            )
            .unwrap();
        fixture
            .store
            .connect_remote(stored, &connection, encrypted)
            .await
            .unwrap();
        connection
    }

    #[rstest]
    #[tokio::test]
    async fn official_service_keeps_failed_revocations_for_retry(#[future] fixture: Fixture) {
        let fixture = fixture.await;
        let old = remote::RemoteCredentials {
            connection_id: uuid::Uuid::new_v4().to_string(),
            capability: "a".repeat(64),
        };
        let new = remote::RemoteCredentials {
            connection_id: uuid::Uuid::new_v4().to_string(),
            capability: "b".repeat(64),
        };
        save_remote(&fixture, &old).await;
        let connection = save_remote(&fixture, &new).await;
        let app = super::router(
            authentication(fixture.store.0.clone()),
            fixture.store.clone(),
            GitHubSettings::new(
                None,
                "https://customer.example".parse().unwrap(),
                ADMIN,
                Some(fixture.provider.uri().parse().unwrap()),
                false,
            )
            .unwrap(),
        );
        Mock::given(method("DELETE"))
            .and(path(format!(
                "/lens/github/service/connections/{}",
                old.connection_id
            )))
            .respond_with(ResponseTemplate::new(503))
            .mount(&fixture.provider)
            .await;
        let response = app
            .clone()
            .oneshot(request(
                "GET",
                "/lens/github/status?agent=agent",
                Some(&token("team-a", "alice", "team")),
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(body(response).await["connection"]["available"], false);
        assert_eq!(
            fixture
                .store
                .retired_remote_credentials("team-a", "agent")
                .await
                .unwrap()
                .len(),
            1
        );
        fixture.provider.reset().await;
        Mock::given(method("DELETE"))
            .and(path(format!(
                "/lens/github/service/connections/{}",
                old.connection_id
            )))
            .and(match_header(
                "authorization",
                format!("Bearer {}", old.capability),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"disconnected":true})))
            .expect(1)
            .mount(&fixture.provider)
            .await;
        Mock::given(method("GET")).and(path(format!("/lens/github/service/connections/{}", new.connection_id)))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"connection":connection,"app_slug":"lens-test","available":true,"availability_error":""}))).expect(1).mount(&fixture.provider).await;
        let response = app
            .oneshot(request(
                "GET",
                "/lens/github/status?agent=agent",
                Some(&token("team-a", "alice", "team")),
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(body(response).await["connection"]["available"], true);
        assert!(
            fixture
                .store
                .retired_remote_credentials("team-a", "agent")
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[rstest]
    #[tokio::test]
    async fn official_service_allows_recovery_after_local_key_rotation(#[future] fixture: Fixture) {
        let fixture = fixture.await;
        let credential = remote::RemoteCredentials {
            connection_id: uuid::Uuid::new_v4().to_string(),
            capability: "a".repeat(64),
        };
        save_remote(&fixture, &credential).await;
        let app = super::router(
            authentication(fixture.store.0.clone()),
            fixture.store.clone(),
            GitHubSettings::new(
                None,
                "https://customer.example".parse().unwrap(),
                "rotated-admin-secret-with-at-least-32-bytes",
                Some(fixture.provider.uri().parse().unwrap()),
                false,
            )
            .unwrap(),
        );
        let response = app
            .clone()
            .oneshot(request(
                "GET",
                "/lens/github/status?agent=agent",
                Some(&token("team-a", "alice", "team")),
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let response = body(response).await;
        assert_eq!(response["configured"], true);
        assert_eq!(response["connection"]["available"], false);
        assert!(
            response["connection"]["availability_error"]
                .as_str()
                .unwrap()
                .contains("Reconnect")
        );
        let response = app
            .oneshot(request(
                "DELETE",
                "/lens/github/connections/agent",
                Some(&token("team-a", "alice", "team")),
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 204);
        assert!(
            fixture
                .store
                .connection("team-a", "agent")
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            fixture
                .store
                .retired_remote_credentials("team-a", "agent")
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(
            fixture
                .provider
                .received_requests()
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[rstest]
    #[tokio::test]
    async fn official_service_handoff_requires_browser_consent_and_stores_scoped_credentials(
        #[future] fixture: Fixture,
    ) {
        let fixture = fixture.await;
        let broker = broker(&fixture).await;
        let (started, local_cookie) = local_start(&broker).await;
        let landing: Url = started["authorization_url"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let id = landing.path().rsplit('/').next().unwrap();
        let response = broker
            .app
            .clone()
            .oneshot(request("GET", landing.path(), None, None, None))
            .await
            .unwrap();
        assert_eq!(response.status(), 303);
        let broker_cookie = cookies(&response);
        let install: Url = response.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(install.path(), "/apps/lens-test/installations/new");
        let state = install
            .query_pairs()
            .find(|(key, _)| key == "state")
            .unwrap()
            .1
            .into_owned();
        let exposed_cookie = format!("lens_github_state={state}");
        let second_browser = broker
            .app
            .clone()
            .oneshot(request(
                "GET",
                landing.path(),
                None,
                Some(&exposed_cookie),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(second_browser.status(), 403);
        assert!(!second_browser.headers().contains_key(header::SET_COOKIE));
        let setup = format!("/lens/github/setup?state={state}&installation_id=999");
        let response = broker
            .app
            .clone()
            .oneshot(request("GET", &setup, None, Some(&broker_cookie), None))
            .await
            .unwrap();
        assert_eq!(response.status(), 303);
        let oauth = response.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .to_owned();
        assert!(oauth.contains("code_challenge="));
        mock_oauth(&fixture.provider, &oauth).await;
        let callback = format!("/lens/github/callback?state={state}&code=one-time-code");
        let response = broker
            .app
            .clone()
            .oneshot(request("GET", &callback, None, Some(&broker_cookie), None))
            .await
            .unwrap();
        assert_eq!(response.headers()[header::LOCATION], landing.as_str());
        let response = broker
            .app
            .clone()
            .oneshot(request(
                "GET",
                landing.path(),
                None,
                Some(&broker_cookie),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let html = String::from_utf8(
            to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        assert!(html.contains("https://customer.example"));
        assert!(html.contains("org/repo"));
        assert!(!html.contains("org/read-only"));
        assert!(!html.contains("transient-user-token"));
        if let Ok(path) = std::env::var("LENS_TEST_CONSENT_HTML") {
            std::fs::write(path, &html).unwrap();
        }
        let unauthorized = broker
            .app
            .clone()
            .oneshot(form(
                landing.path(),
                &broker.origin,
                &exposed_cookie,
                &state,
            ))
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), 403);
        let foreign_origin: Url = "https://attacker.example".parse().unwrap();
        let csrf = broker
            .app
            .clone()
            .oneshot(form(
                landing.path(),
                &foreign_origin,
                &broker_cookie,
                &state,
            ))
            .await
            .unwrap();
        assert_eq!(csrf.status(), 403);
        mock_installation(&fixture.provider, 10, 200).await;
        let response = broker
            .app
            .clone()
            .oneshot(form(landing.path(), &broker.origin, &broker_cookie, &state))
            .await
            .unwrap();
        assert_eq!(response.status(), 303);
        let callback: Url = response.headers()[header::LOCATION]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            callback.origin().ascii_serialization(),
            "https://customer.example"
        );
        assert_eq!(callback.path(), remote::CALLBACK_PATH);
        let code = callback
            .query_pairs()
            .find(|(key, _)| key == "code")
            .unwrap()
            .1
            .into_owned();
        let wrong = broker
            .app
            .clone()
            .oneshot(request(
                "POST",
                "/lens/github/service/redeem",
                None,
                None,
                Some(json!({"code":code,"code_verifier":"x".repeat(43)})),
            ))
            .await
            .unwrap();
        assert_eq!(wrong.status(), 403);
        let response = broker
            .local
            .clone()
            .oneshot(request(
                "GET",
                &format!("{}?{}", callback.path(), callback.query().unwrap()),
                None,
                Some(&local_cookie),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 303);
        assert!(
            !response.headers()[header::LOCATION]
                .to_str()
                .unwrap()
                .contains("code=")
        );
        let status = broker
            .local
            .clone()
            .oneshot(request(
                "GET",
                "/lens/github/status?agent=agent",
                Some(&token("team-a", "alice", "team")),
                None,
                None,
            ))
            .await
            .unwrap();
        let status = body(status).await;
        assert_eq!(status["configured"], true);
        assert_eq!(
            status["service_origin"],
            broker.origin.origin().ascii_serialization()
        );
        assert_eq!(status["connection"]["available"], true);
        let encrypted = fixture
            .store
            .remote_credentials("team-a", "agent")
            .await
            .unwrap()
            .unwrap();
        let cipher = CredentialCipher::new(ADMIN).unwrap();
        let credential: remote::RemoteCredentials = cipher
            .decrypt(
                &encrypted,
                &litellm_storage_clickhouse::github::remote_credentials_key("team-a", "agent"),
            )
            .unwrap();
        assert!(!encrypted.to_string().contains(&credential.capability));
        let saved = fixture
            .store
            .broker_connection(&credential.connection_id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            !serde_json::to_string(&saved.connection)
                .unwrap()
                .contains(&credential.capability)
        );
        assert_eq!(saved.connection.connection.repository_id, 10);
        assert_eq!(saved.connection.lens_origin, "https://customer.example");
        let run = completed_run(&fixture).await;
        reporting_provider(&fixture.provider).await;
        let response = broker
            .local
            .clone()
            .oneshot(request(
                "POST",
                "/lens/github/report",
                Some(&token("team-a", "alice", "team")),
                None,
                Some(json!({"run_ids":[run.id]})),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 200, "{}", body(response).await);
        let repeated = broker
            .local
            .clone()
            .oneshot(request(
                "POST",
                "/lens/github/report",
                Some(&token("team-a", "alice", "team")),
                None,
                Some(json!({"run_ids":[run.id]})),
            ))
            .await
            .unwrap();
        assert_eq!(repeated.status(), 200);
        assert_eq!(body(repeated).await["reports"][0]["run_id"], run.id);
        let foreign_team = broker
            .local
            .clone()
            .oneshot(request(
                "POST",
                "/lens/github/report",
                Some(&token("team-b", "bob", "team")),
                None,
                Some(json!({"run_ids":[run.id]})),
            ))
            .await
            .unwrap();
        assert_eq!(foreign_team.status(), 400);
        let mut foreign_run = run.clone();
        foreign_run.agent = "another-agent".into();
        let foreign_report = broker.app.clone().oneshot(request("POST", &format!("/lens/github/service/connections/{}/report",credential.connection_id), Some(&credential.capability), None, Some(json!({"run":foreign_run,"baseline":null,"ci_url":"https://github.com/org/renamed/actions/runs/50"})))).await.unwrap();
        assert_eq!(foreign_report.status(), 403);
        let replay = broker.app.clone().oneshot(request("POST", "/lens/github/service/redeem", None, None, Some(json!({"code":code,"code_verifier":cipher.verifier(started["authorization_id"].as_str().unwrap())})))).await.unwrap();
        assert_eq!(replay.status(), 410);
        let foreign = broker
            .app
            .clone()
            .oneshot(request(
                "GET",
                &format!("/lens/github/service/connections/{}", uuid::Uuid::new_v4()),
                Some(&credential.capability),
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(foreign.status(), 403);
        let wrong = broker
            .app
            .clone()
            .oneshot(request(
                "GET",
                &format!(
                    "/lens/github/service/connections/{}",
                    credential.connection_id
                ),
                Some(&"0".repeat(64)),
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(wrong.status(), 403);
        let auth = fixture.store.authorization(id).await.unwrap().unwrap();
        assert_eq!(auth.authorization.state, AuthorizationState::Connected);
        let response = broker
            .local
            .clone()
            .oneshot(request(
                "DELETE",
                "/lens/github/connections/agent",
                Some(&token("team-a", "alice", "team")),
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 204);
        assert!(
            fixture
                .store
                .connection("team-a", "agent")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            fixture
                .store
                .remote_credentials("team-a", "agent")
                .await
                .unwrap()
                .is_none()
        );
        let revoked = broker
            .app
            .clone()
            .oneshot(request(
                "GET",
                &format!(
                    "/lens/github/service/connections/{}",
                    credential.connection_id
                ),
                Some(&credential.capability),
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(revoked.status(), 403);
        fixture.provider.verify().await;
    }

    #[rstest]
    #[tokio::test]
    async fn official_service_browser_claim_has_one_winner(#[future] fixture: Fixture) {
        let fixture = fixture.await;
        let broker = broker(&fixture).await;
        let (started, _) = local_start(&broker).await;
        let url: Url = started["authorization_url"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let (first, second) = tokio::join!(
            broker
                .app
                .clone()
                .oneshot(request("GET", url.path(), None, None, None)),
            broker
                .app
                .clone()
                .oneshot(request("GET", url.path(), None, None, None))
        );
        let first = first.unwrap();
        let second = second.unwrap();
        assert_eq!(
            usize::from(first.status() == 303) + usize::from(second.status() == 303),
            1
        );
        let rejected = if first.status() == 303 { second } else { first };
        assert!(matches!(rejected.status().as_u16(), 403 | 409));
        assert!(!rejected.headers().contains_key(header::SET_COOKIE));
    }

    #[rstest]
    #[case::remote_http("http://customer.example/lens/github/service/callback")]
    #[case::userinfo("https://user@customer.example/lens/github/service/callback")]
    #[case::different_path("https://customer.example/somewhere")]
    #[case::query("https://customer.example/lens/github/service/callback?redirect=elsewhere")]
    #[case::fragment("https://customer.example/lens/github/service/callback#secret")]
    #[tokio::test]
    async fn official_service_rejects_unsafe_callback_before_storage(#[case] redirect_uri: &str) {
        use base64::Engine;
        let provider = MockServer::start().await;
        let state = ClickHouseState::new(
            Client::no_redirect_for_test(),
            DatabaseConnection::reader("http://127.0.0.1:9", "unreachable").unwrap(),
        );
        let public: Url = "https://service.example".parse().unwrap();
        let mut github = github(&provider);
        github.public_url = public.clone();
        let app = super::router(
            authentication(state.clone()),
            GitHubStore(state),
            GitHubSettings::new(Some(github), public, ADMIN, None, true).unwrap(),
        );
        let response = app.oneshot(request("POST", "/lens/github/service/authorize", None, None, Some(json!({"redirect_uri":redirect_uri,"state":"browser-state","agent":"agent","code_challenge":base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest("verifier"))})))).await.unwrap();
        assert_eq!(response.status(), 400);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert!(provider.received_requests().await.unwrap().is_empty());
    }

    #[rstest]
    #[tokio::test]
    async fn official_service_expired_handoff_cannot_claim_browser_or_redeem(
        #[future] fixture: Fixture,
    ) {
        let fixture = fixture.await;
        let broker = broker(&fixture).await;
        let auth = Authorization {
            id: uuid::Uuid::new_v4().to_string(),
            owner: Owner {
                scope: "team-a".into(),
                subject: "alice".into(),
            },
            agent: "agent".into(),
            expires_at: Utc::now() - TimeDelta::minutes(1),
            state: AuthorizationState::Pending,
        };
        let handshake = lens_contract::github::BrokerHandshake {
            id: auth.id.clone(),
            lens_origin: "https://customer.example".into(),
            redirect_uri: "https://customer.example/lens/github/service/callback".into(),
            local_state: "state".into(),
            code_challenge: "challenge".into(),
            agent: auth.agent.clone(),
            expires_at: auth.expires_at,
            browser_claimed: false,
            browser_hash: String::new(),
            state: lens_contract::github::BrokerHandshakeState::Pending,
        };
        fixture
            .store
            .create_handshake(&auth, &handshake)
            .await
            .unwrap();
        let response = broker
            .app
            .clone()
            .oneshot(request(
                "GET",
                &format!("/lens/github/service/connect/{}", auth.id),
                None,
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 410);
        assert!(!response.headers().contains_key(header::SET_COOKIE));
        let response = broker.app.clone().oneshot(request("POST", "/lens/github/service/redeem", None, None, Some(json!({"code":format!("{}.{}",auth.id,"a".repeat(64)),"code_verifier":"a".repeat(43)})))).await.unwrap();
        assert_eq!(response.status(), 410);
    }

    fn router(
        authentication: Arc<Authentication<Sessions>>,
        store: GitHubStore,
        github: Option<GitHubApp>,
    ) -> Router {
        super::router(
            authentication,
            store,
            GitHubSettings::new(
                github,
                "http://lens.test".parse().unwrap(),
                ADMIN,
                None,
                false,
            )
            .unwrap(),
        )
    }

    fn github(provider: &MockServer) -> GitHubApp {
        GitHubApp::new(
            "lens-test".into(),
            "client-id".into(),
            "client-secret".into(),
            include_str!("../tests/fixtures/github-test-key.pem"),
            "http://lens.test".parse().unwrap(),
        )
        .unwrap()
        .with_endpoints(&provider.uri())
    }

    fn authentication(state: ClickHouseState) -> Arc<Authentication<Sessions>> {
        Arc::new(Authentication {
            settings: Settings::new(ADMIN, Some(SECRET.into()), "http://lens.test").unwrap(),
            sessions: Sessions(state),
        })
    }

    fn token(team: &str, subject: &str, role: &str) -> String {
        let now = Utc::now().timestamp();
        encode(&Header::default(), &json!({"iss":"litellm","aud":"litellm-lens","sub":subject,"iat":now,"exp":now+60,
            "identity":{"user_role":role,"user_id":subject,"team_id":team,"org_id":null,"token":subject,"models":[],"log_team_ids":[]}}), &EncodingKey::from_secret(SECRET.as_bytes())).unwrap()
    }

    fn request(
        method: &str,
        uri: &str,
        token: Option<&str>,
        cookie: Option<&str>,
        body: Option<Value>,
    ) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        if let Some(cookie) = cookie {
            builder = builder.header("cookie", cookie);
        }
        builder
            .body(body.map_or_else(Body::empty, |value| Body::from(value.to_string())))
            .unwrap()
    }

    async fn body(response: Response) -> Value {
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
    }

    struct Fixture {
        app: Router,
        store: GitHubStore,
        provider: MockServer,
        _container: ContainerAsync<ClickHouse>,
    }

    #[fixture]
    async fn fixture() -> Fixture {
        let container = ClickHouse::default()
            .with_tag(
                "26.9.6.6@sha256:eb4870e7ca7ed70c259eebfcfbee6cf797017f6b5436c2926bbbfe3d4d28486e",
            )
            .with_env_var("CLICKHOUSE_SKIP_USER_SETUP", "1")
            .with_copy_to(
                "/etc/clickhouse-server/config.d/lens-keeper.xml",
                include_bytes!("../../storage-clickhouse/tests/state/fixtures/keeper.xml").to_vec(),
            )
            .start()
            .await
            .unwrap();
        let url = format!(
            "http://{}:{}",
            container.get_host().await.unwrap(),
            container.get_host_port_ipv4(8123).await.unwrap()
        );
        let database = format!("github_{}", uuid::Uuid::new_v4().simple());
        execute_statement(
            &Client::no_redirect_for_test(),
            &DatabaseConnection::writer(&url).unwrap(),
            &format!("CREATE DATABASE `{database}`"),
            Duration::from_secs(30),
        )
        .await
        .unwrap();
        let state = ClickHouseState::new(
            Client::no_redirect_for_test(),
            DatabaseConnection::reader(&url, &database).unwrap(),
        );
        state
            .initialize(&format!("/github-tests/{database}"))
            .await
            .unwrap();
        let provider = MockServer::start().await;
        let app = router(
            authentication(state.clone()),
            GitHubStore(state.clone()),
            Some(github(&provider)),
        );
        Fixture {
            app,
            store: GitHubStore(state),
            provider,
            _container: container,
        }
    }

    async fn start(fixture: &Fixture, install: bool) -> (String, String, String) {
        let response = fixture
            .app
            .clone()
            .oneshot(request(
                "POST",
                "/lens/github/authorize",
                Some(&token("team-a", "alice", "team")),
                None,
                Some(json!({"agent":"agent", "install":install})),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let cookie = response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        let value = body(response).await;
        (
            value["authorization_id"].as_str().unwrap().into(),
            cookie,
            value["authorization_url"].as_str().unwrap().into(),
        )
    }

    async fn mock_oauth(provider: &MockServer, authorization_url: &str) {
        let url = url::Url::parse(authorization_url).unwrap();
        let redirect_uri = url
            .query_pairs()
            .find(|(key, _)| key == "redirect_uri")
            .unwrap()
            .1
            .into_owned();
        let challenge = url
            .query_pairs()
            .find(|(key, _)| key == "code_challenge")
            .unwrap()
            .1
            .into_owned();
        Mock::given(method("POST"))
            .and(path("/login/oauth/access_token"))
            .and(move |request: &wiremock::Request| {
                use base64::Engine;
                let form = url::form_urlencoded::parse(&request.body)
                    .into_owned()
                    .collect::<std::collections::HashMap<_, _>>();
                form.get("code").is_some_and(|code| code == "one-time-code")
                    && form
                        .get("redirect_uri")
                        .is_some_and(|url| url == &redirect_uri)
                    && form.get("code_verifier").is_some_and(|verifier| {
                        base64::engine::general_purpose::URL_SAFE_NO_PAD
                            .encode(Sha256::digest(verifier))
                            == challenge
                    })
            })
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"access_token":"transient-user-token"})),
            )
            .expect(1)
            .mount(provider)
            .await;
        for (route, response) in [
            ("/user", json!({"id":7})),
            ("/user/installations", json!({"installations":[{"id":42}]})),
            (
                "/user/installations/42/repositories",
                json!({"repositories":[
                    {"id":10,"full_name":"org/repo","default_branch":"main","permissions":{"push":true}},
                    {"id":11,"full_name":"org/read-only","default_branch":"main","permissions":{"pull":true}}
                ]}),
            ),
        ] {
            Mock::given(method("GET"))
                .and(path(route))
                .and(match_header("authorization", "Bearer transient-user-token"))
                .respond_with(ResponseTemplate::new(200).set_body_json(response))
                .expect(1)
                .mount(provider)
                .await;
        }
    }

    async fn mock_installation(provider: &MockServer, id: u64, status: u16) {
        Mock::given(method("POST"))
            .and(path("/app/installations/42/access_tokens"))
            .and(body_partial_json(json!({"repository_ids":[10]})))
            .respond_with(
                ResponseTemplate::new(201)
                    .set_body_json(json!({"token":"scoped-installation-token"})),
            )
            .mount(provider)
            .await;
        Mock::given(method("GET")).and(path("/installation/repositories"))
            .and(match_header("authorization", "Bearer scoped-installation-token"))
            .respond_with(ResponseTemplate::new(status).set_body_json(json!({"repositories":[{"id":id,"full_name":"org/renamed","default_branch":"trunk"}]}))).mount(provider).await;
    }

    #[rstest]
    #[tokio::test]
    async fn oauth_connects_only_verified_writable_repositories_and_isolates_owners(
        #[future] fixture: Fixture,
    ) {
        let fixture = fixture.await;
        let (id, cookie, url) = start(&fixture, false).await;
        mock_oauth(&fixture.provider, &url).await;
        let state = cookie.strip_prefix("lens_github_state=").unwrap();
        let callback = format!("/lens/github/callback?state={state}&code=one-time-code");
        let response = fixture
            .app
            .clone()
            .oneshot(request("GET", &callback, None, Some(&cookie), None))
            .await
            .unwrap();
        assert_eq!(response.status(), 303);
        assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
        let redirect = response.headers()[header::LOCATION].to_str().unwrap();
        assert!(redirect.starts_with(
            "http://lens.test/ui/?tab=agents&github_agent=agent&github_authorization="
        ));
        assert!(!redirect.contains("code="));
        let poll = format!("/lens/github/authorizations/{id}");
        let alice = token("team-a", "alice", "team");
        let response = fixture
            .app
            .clone()
            .oneshot(request("GET", &poll, Some(&alice), None, None))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(
            body(response).await,
            json!({"status":"ready","repositories":[{"id":10,"full_name":"org/repo","installation_id":42,"default_branch":"main"}]})
        );
        let stored = serde_json::to_string(
            &fixture
                .store
                .authorization(&id)
                .await
                .unwrap()
                .unwrap()
                .authorization,
        )
        .unwrap();
        assert!(!stored.contains("transient-user-token"));
        assert!(!stored.contains("one-time-code"));
        for outsider in [
            token("team-b", "alice", "team"),
            token("team-a", "bob", "team"),
        ] {
            let response = fixture
                .app
                .clone()
                .oneshot(request("GET", &poll, Some(&outsider), None, None))
                .await
                .unwrap();
            assert_eq!(response.status(), 403);
            let response = fixture
                .app
                .clone()
                .oneshot(request(
                    "PUT",
                    "/lens/github/connections/agent",
                    Some(&outsider),
                    None,
                    Some(json!({"authorization_id":id,"repository_id":10})),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), 403);
        }
        for (agent, repository_id) in [("other", 10), ("agent", 11), ("agent", 999)] {
            let response = fixture
                .app
                .clone()
                .oneshot(request(
                    "PUT",
                    &format!("/lens/github/connections/{agent}"),
                    Some(&alice),
                    None,
                    Some(json!({"authorization_id":id,"repository_id":repository_id})),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), 403);
        }
        mock_installation(&fixture.provider, 10, 200).await;
        let connect_request = || {
            request(
                "PUT",
                "/lens/github/connections/agent",
                Some(&alice),
                None,
                Some(json!({"authorization_id":id,"repository_id":10})),
            )
        };
        let (first, second) = tokio::join!(
            fixture.app.clone().oneshot(connect_request()),
            fixture.app.clone().oneshot(connect_request())
        );
        let statuses = [first.unwrap().status(), second.unwrap().status()];
        assert_eq!(
            statuses
                .iter()
                .filter(|status| **status == StatusCode::OK)
                .count(),
            1
        );
        assert!(
            statuses
                .iter()
                .any(|status| *status == StatusCode::CONFLICT || *status == StatusCode::GONE)
        );
        let saved = fixture
            .store
            .connection("team-a", "agent")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                saved.repository_id,
                saved.installation_id,
                saved.repository.as_str()
            ),
            (10, 42, "org/renamed")
        );
        assert!(
            fixture
                .store
                .connection("team-b", "agent")
                .await
                .unwrap()
                .is_none()
        );
        let response = fixture
            .app
            .clone()
            .oneshot(request(
                "GET",
                "/lens/github/status?agent=agent",
                Some(&alice),
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(body(response).await["connection"]["available"], true);
        let replay = fixture
            .app
            .clone()
            .oneshot(request("GET", &callback, None, Some(&cookie), None))
            .await
            .unwrap();
        assert_eq!(replay.status(), 303);
        let response = fixture
            .app
            .clone()
            .oneshot(request("GET", &poll, Some(&alice), None, None))
            .await
            .unwrap();
        assert_eq!(
            body(response).await,
            json!({"status":"connected","repositories":[]})
        );
        fixture.provider.verify().await;
        fixture.provider.reset().await;
        mock_installation(&fixture.provider, 999, 200).await;
        let response = fixture
            .app
            .clone()
            .oneshot(request(
                "GET",
                "/lens/github/status?agent=agent",
                Some(&alice),
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(body(response).await["connection"]["available"], false);
        let foreign = Authorization {
            id: uuid::Uuid::new_v4().to_string(),
            owner: Owner {
                scope: "team-a".into(),
                subject: "alice".into(),
            },
            agent: "second".into(),
            expires_at: Utc::now() + TimeDelta::minutes(10),
            state: AuthorizationState::Ready {
                repositories: vec![lens_contract::github::Repository {
                    id: 10,
                    full_name: "org/repo".into(),
                    installation_id: 42,
                    default_branch: "main".into(),
                }],
            },
        };
        fixture.store.create_authorization(&foreign).await.unwrap();
        let response = fixture
            .app
            .clone()
            .oneshot(request(
                "PUT",
                "/lens/github/connections/second",
                Some(&alice),
                None,
                Some(json!({"authorization_id":foreign.id,"repository_id":10})),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 403);
        assert!(
            fixture
                .store
                .connection("team-a", "second")
                .await
                .unwrap()
                .is_none()
        );
        let response = fixture
            .app
            .clone()
            .oneshot(request(
                "DELETE",
                "/lens/github/connections/agent",
                Some(&alice),
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 204);
        assert!(
            fixture
                .store
                .connection("team-a", "agent")
                .await
                .unwrap()
                .is_none()
        );
        fixture.provider.verify().await;
    }

    #[rstest]
    #[tokio::test]
    async fn installation_callbacks_are_browser_bound_one_use_and_fail_safely(
        #[future] fixture: Fixture,
    ) {
        let fixture = fixture.await;
        let (id, cookie, url) = start(&fixture, true).await;
        assert!(url.contains("/apps/lens-test/installations/new?state="));
        let state = cookie.strip_prefix("lens_github_state=").unwrap();
        let setup = format!("/lens/github/setup?state={state}&installation_id=9999");
        for cookie in [None, Some("lens_github_state=wrong")] {
            let response = fixture
                .app
                .clone()
                .oneshot(request("GET", &setup, None, cookie, None))
                .await
                .unwrap();
            assert_eq!(response.status(), 400);
            assert!(
                response.headers()[header::CONTENT_TYPE]
                    .to_str()
                    .unwrap()
                    .starts_with("text/html")
            );
        }
        let response = fixture
            .app
            .clone()
            .oneshot(request("GET", &setup, None, Some(&cookie), None))
            .await
            .unwrap();
        assert_eq!(response.status(), 303);
        assert!(
            response.headers()[header::LOCATION]
                .to_str()
                .unwrap()
                .contains("/login/oauth/authorize?")
        );
        assert_eq!(
            fixture
                .store
                .authorization(&id)
                .await
                .unwrap()
                .unwrap()
                .authorization
                .state,
            AuthorizationState::Pending
        );
        let response = fixture
            .app
            .clone()
            .oneshot(request(
                "GET",
                &format!("/lens/github/callback?state={state}&error=access_denied"),
                None,
                Some(&cookie),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 303);
        let stored = fixture
            .store
            .authorization(&id)
            .await
            .unwrap()
            .unwrap()
            .authorization;
        assert!(matches!(stored.state, AuthorizationState::Failed { .. }));
        assert!(
            fixture
                .store
                .connection("team-a", "agent")
                .await
                .unwrap()
                .is_none()
        );
        let expired = Authorization {
            id: uuid::Uuid::new_v4().to_string(),
            expires_at: Utc::now() - TimeDelta::seconds(1),
            state: AuthorizationState::Pending,
            ..stored
        };
        fixture.store.create_authorization(&expired).await.unwrap();
        let expired_state = github(&fixture.provider).state(&expired.id);
        let response = fixture
            .app
            .clone()
            .oneshot(request(
                "GET",
                &format!("/lens/github/callback?state={expired_state}&code=unused"),
                None,
                Some(&format!("lens_github_state={expired_state}")),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 303);
        let response = fixture
            .app
            .clone()
            .oneshot(request(
                "GET",
                &format!("/lens/github/authorizations/{}", expired.id),
                Some(&token("team-a", "alice", "team")),
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 410);
        assert!(
            fixture
                .provider
                .received_requests()
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[rstest]
    #[tokio::test]
    async fn browser_writes_require_same_origin_and_share_admin_bearer_ownership(
        #[future] fixture: Fixture,
    ) {
        let fixture = fixture.await;
        authentication(fixture.store.0.clone())
            .sign_in(ADMIN, "browser-session", Utc::now())
            .await
            .unwrap();
        for origin in [None, Some("https://foreign.test")] {
            let mut request = request(
                "POST",
                "/lens/github/authorize",
                None,
                Some("lens_session=browser-session"),
                Some(json!({"agent":"agent"})),
            );
            if let Some(origin) = origin {
                request
                    .headers_mut()
                    .insert(header::ORIGIN, origin.parse().unwrap());
            }
            let response = fixture.app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), 403);
        }
        let mut request = request(
            "POST",
            "/lens/github/authorize",
            None,
            Some("lens_session=browser-session"),
            Some(json!({"agent":"agent"})),
        );
        request
            .headers_mut()
            .insert(header::ORIGIN, "http://lens.test".parse().unwrap());
        let response = fixture.app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), 200);
        let value = body(response).await;
        let uri = format!(
            "/lens/github/authorizations/{}",
            value["authorization_id"].as_str().unwrap()
        );
        let response = fixture
            .app
            .clone()
            .oneshot(self::request("GET", &uri, Some(ADMIN), None, None))
            .await
            .unwrap();
        assert_eq!(
            body(response).await,
            json!({"status":"pending","repositories":[]})
        );
        let unconfigured = router(
            authentication(fixture.store.0.clone()),
            fixture.store.clone(),
            None,
        );
        let response = unconfigured
            .clone()
            .oneshot(self::request(
                "GET",
                "/lens/github/status?agent=agent",
                Some(ADMIN),
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(
            body(response).await,
            json!({"configured":false,"app_slug":null,"connection":null})
        );
        let response = unconfigured
            .oneshot(self::request(
                "POST",
                "/lens/github/authorize",
                Some(ADMIN),
                None,
                Some(json!({"agent":"agent"})),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), 503);
    }

    #[rstest]
    #[case("POST", "/lens/github/authorize", json!({"agent":"agent"}))]
    #[case("PUT", "/lens/github/connections/agent", json!({"authorization_id":"x","repository_id":1}))]
    #[case("DELETE", "/lens/github/connections/agent", Value::Null)]
    #[case("POST", "/lens/github/report", json!({"run_ids":["run"]}))]
    #[tokio::test]
    async fn writes_require_authenticated_non_viewer_identity(
        #[case] method: &str,
        #[case] path: &str,
        #[case] payload: Value,
    ) {
        let state = ClickHouseState::new(
            Client::no_redirect_for_test(),
            DatabaseConnection::reader("http://127.0.0.1:9", "unreachable").unwrap(),
        );
        let app = router(authentication(state.clone()), GitHubStore(state), None);
        for role in ["internal_user_viewer", "proxy_admin_viewer", "customer"] {
            let response = app
                .clone()
                .oneshot(request(
                    method,
                    path,
                    Some(&token("team-a", "user", role)),
                    None,
                    Some(payload.clone()),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), 403);
        }
        let response = app
            .oneshot(request(method, path, None, None, Some(payload)))
            .await
            .unwrap();
        assert_eq!(response.status(), 401);
    }
}
