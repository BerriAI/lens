use std::sync::Arc;

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use chrono::{DateTime, Utc};
use lens_auth::SessionRepository;
use lens_contract::{
    eval::{EvalRun, RunStatus},
    github::Connection,
};
use litellm_storage_clickhouse::evals::EvalStore;
use reqwest::Method;
use serde::{Deserialize, Serialize};

use super::{App, GitHubApp, GitHubError, owner, remote, report, workflow};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    run_id: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    pub run: EvalRun,
    pub ci_url: String,
}

#[derive(Serialize, Deserialize)]
pub(super) struct Requested {
    pub ci_url: String,
    pub requested_at: DateTime<Utc>,
}

pub(super) async fn start<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
    Json(request): Json<Request>,
) -> Result<(StatusCode, Json<Requested>), GitHubError> {
    let identity = crate::auth::identity(&app.authentication, &headers, &Method::POST).await?;
    let owner = owner(&identity, true)?;
    if request.run_id.is_empty() || request.run_id.len() > 200 {
        return Err(GitHubError::Invalid("Choose a saved Lens eval run"));
    }
    let stored = EvalStore::new(app.store.0.clone())
        .get(&owner.scope, &request.run_id)
        .await
        .map_err(report::eval_error)?;
    let connection = app
        .store
        .connection(&owner.scope, &stored.run.agent)
        .await?
        .ok_or(GitHubError::Invalid(
            "Connect this agent to its GitHub repository before running again",
        ))?;
    let input = Input {
        run: stored.run,
        ci_url: stored.request.ci_url,
    };
    let requested = match (&app.settings.remote, &app.settings.github) {
        (Some(remote), _) => {
            remote::cleanup_retired(&app, remote, &owner.scope, &input.run.agent).await?;
            let credentials = remote::credentials(&app, &owner.scope, &input.run.agent)
                .await?
                .ok_or(GitHubError::Credentials)?;
            remote.rerun(&credentials, input).await?
        }
        (_, Some(github)) => request_rerun(github, &connection, input).await?,
        _ => return Err(GitHubError::NotConfigured),
    };
    Ok((StatusCode::ACCEPTED, Json(requested)))
}

pub(super) async fn request_rerun(
    github: &GitHubApp,
    connection: &Connection,
    input: Input,
) -> Result<Requested, GitHubError> {
    if input.run.agent != connection.agent {
        return Err(GitHubError::Forbidden);
    }
    if !matches!(input.run.status, RunStatus::Done | RunStatus::Failed) {
        return Err(GitHubError::Invalid(
            "Wait for this eval run to finish before running it again",
        ));
    }
    let token = github.installation_token(connection).await?;
    let verified = workflow::verify(github, &token, connection, &input.run, &input.ci_url).await?;
    if verified.status != "completed" {
        return Err(GitHubError::Invalid(
            "The GitHub workflow is still running. Wait for it to finish before running again",
        ));
    }
    let target = verified.target;
    github.post_empty(&format!("{}/actions/runs/{}/rerun", target.path, target.id), &token)
        .await
        .map_err(|error| match error {
            GitHubError::Upstream { status: 403 } => GitHubError::Invalid(
                "GitHub denied the rerun. Grant the Lens GitHub App Actions write access, then approve the updated installation permissions",
            ),
            GitHubError::Upstream { status: 422 } => GitHubError::Invalid(
                "GitHub cannot rerun this workflow. It may already be running or outside GitHub's rerun window. Open the workflow in GitHub for details",
            ),
            error => error,
        })?;
    Ok(Requested {
        ci_url: format!(
            "https://github.com{}/actions/runs/{}",
            target.path.strip_prefix("/repos").expect("repository path"),
            target.id
        ),
        requested_at: Utc::now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::{fixture, rstest};
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, header, method, path},
    };

    #[fixture]
    fn connection() -> Connection {
        Connection {
            agent: "demo".into(),
            repository_id: 9,
            repository: "org/repo".into(),
            installation_id: 3,
            default_branch: "main".into(),
            connected_at: Utc::now(),
        }
    }

    #[fixture]
    fn input() -> Input {
        let mut run: EvalRun = serde_json::from_str(include_str!(
            "../../../../../sdk/tests/fixtures/lens_eval/eval_run_no_baseline.json"
        ))
        .unwrap();
        run.agent = "demo".into();
        run.version = "a".repeat(40);
        run.pr = None;
        Input {
            run,
            ci_url: "https://github.com/org/repo/actions/runs/123".into(),
        }
    }

    fn github(server: &MockServer) -> GitHubApp {
        GitHubApp::new(
            "lens-test".into(),
            "client".into(),
            "secret".into(),
            include_str!("../../tests/fixtures/github-test-key.pem"),
            "https://lens.example".parse().unwrap(),
        )
        .unwrap()
        .with_endpoints(&server.uri())
    }

    async fn provider(server: &MockServer, status: &str, version: &str, response: u16, calls: u64) {
        Mock::given(method("POST"))
            .and(path("/app/installations/3/access_tokens"))
            .and(body_json(json!({"repository_ids":[9]})))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"token":"scoped-token"})))
            .expect(1)
            .mount(server)
            .await;
        Mock::given(method("GET")).and(path("/installation/repositories"))
            .and(header("authorization", "Bearer scoped-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"repositories":[{"id":9,"full_name":"org/renamed","default_branch":"main"}]})))
            .expect(1).mount(server).await;
        Mock::given(method("GET")).and(path("/repos/org/renamed/actions/runs/123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":123,"status":status,"event":"push","head_sha":version,"repository":{"id":9},"pull_requests":[]})))
            .expect(1).mount(server).await;
        Mock::given(method("POST"))
            .and(path("/repos/org/renamed/actions/runs/123/rerun"))
            .and(header("authorization", "Bearer scoped-token"))
            .respond_with(ResponseTemplate::new(response))
            .expect(calls)
            .mount(server)
            .await;
    }

    #[rstest]
    #[case::accepted(201, None)]
    #[case::permission_denied(403, Some("Actions write"))]
    #[case::expired_or_running(422, Some("rerun window"))]
    #[tokio::test]
    async fn reruns_only_the_verified_workflow_and_accepts_an_empty_response(
        connection: Connection,
        input: Input,
        #[case] response: u16,
        #[case] error_text: Option<&str>,
    ) {
        let server = MockServer::start().await;
        provider(&server, "completed", &input.run.version, response, 1).await;
        let result = request_rerun(&github(&server), &connection, input).await;
        match error_text {
            Some(expected) => assert!(result.err().unwrap().to_string().contains(expected)),
            None => assert_eq!(
                result.unwrap().ci_url,
                "https://github.com/org/renamed/actions/runs/123"
            ),
        }
    }

    #[rstest]
    #[case::in_progress("in_progress", 'a')]
    #[case::queued("queued", 'a')]
    #[case::foreign_commit("completed", 'b')]
    #[tokio::test]
    async fn does_not_rerun_active_or_unrelated_workflows(
        connection: Connection,
        input: Input,
        #[case] status: &str,
        #[case] version: char,
    ) {
        let server = MockServer::start().await;
        provider(&server, status, &version.to_string().repeat(40), 201, 0).await;
        assert!(
            request_rerun(&github(&server), &connection, input)
                .await
                .is_err()
        );
    }

    #[rstest]
    #[case::foreign_agent("other", RunStatus::Done)]
    #[case::running("demo", RunStatus::Running)]
    #[case::scoring("demo", RunStatus::Scoring)]
    #[tokio::test]
    async fn rejects_invalid_runs_before_contacting_github(
        connection: Connection,
        mut input: Input,
        #[case] agent: &str,
        #[case] status: RunStatus,
    ) {
        input.run.agent = agent.into();
        input.run.status = status;
        let server = MockServer::start().await;
        assert!(
            request_rerun(&github(&server), &connection, input)
                .await
                .is_err()
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}
