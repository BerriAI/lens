use std::sync::Arc;

use axum::{Json, extract::State, http::HeaderMap};
use lens_auth::SessionRepository;
use lens_contract::github::{Connection, ProgressPublished, ProgressRequest};
use litellm_storage_clickhouse::{evals::EvalStore, state::ClickHouseState};
use reqwest::Method;
use serde::{Deserialize, Serialize};

use super::{App, GitHubApp, GitHubError, owner, publication, remote, report};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProgressInput {
    pub agent: String,
    pub request: ProgressRequest,
}

pub(super) async fn publish<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
    Json(request): Json<ProgressRequest>,
) -> Result<Json<ProgressPublished>, GitHubError> {
    let identity = crate::auth::identity(&app.authentication, &headers, &Method::POST).await?;
    let owner = owner(&identity, true)?;
    validate(&request)?;
    let definition = EvalStore::new(app.store.0.clone())
        .definition(&owner.scope, &request.name)
        .await
        .map_err(report::eval_error)?;
    let agent = definition.spec.agent;
    let connection =
        app.store
            .connection(&owner.scope, &agent)
            .await?
            .ok_or(GitHubError::Invalid(
                "Connect this agent to a GitHub repository first",
            ))?;
    let input = ProgressInput { agent, request };
    let result = match (&app.settings.remote, &app.settings.github) {
        (Some(remote), _) => {
            remote::cleanup_retired(&app, remote, &owner.scope, &input.agent).await?;
            let credentials = remote::credentials(&app, &owner.scope, &input.agent)
                .await?
                .ok_or(GitHubError::Credentials)?;
            remote.progress(&credentials, input).await?
        }
        (_, Some(github)) => {
            publish_input(github, &app.store.0, &owner.scope, &connection, input).await?
        }
        _ => return Err(GitHubError::NotConfigured),
    };
    Ok(Json(result))
}

fn validate(request: &ProgressRequest) -> Result<(), GitHubError> {
    if !crate::evals::valid_eval_name(&request.name)
        || request.name.len() > 200
        || request.pr == 0
        || request.ci_url.len() > 2048
        || request.version.len() != 40
        || !request.version.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(GitHubError::Invalid(
            "Provide a saved eval name, pull request, commit and GitHub Actions URL",
        ));
    }
    Ok(())
}

pub(super) async fn publish_input(
    github: &GitHubApp,
    state: &ClickHouseState,
    namespace: &str,
    connection: &Connection,
    input: ProgressInput,
) -> Result<ProgressPublished, GitHubError> {
    validate(&input.request)?;
    if input.agent != connection.agent {
        return Err(GitHubError::Forbidden);
    }
    let result = publication::publish(
        github,
        state,
        namespace,
        connection,
        publication::Content::Progress(input),
    )
    .await?;
    Ok(ProgressPublished {
        comment_url: result
            .comment_url
            .ok_or(GitHubError::Invalid("Progress requires a pull request"))?,
        check_url: result.check_url,
    })
}
