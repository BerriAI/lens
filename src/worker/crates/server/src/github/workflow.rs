use lens_contract::github::Connection;
use reqwest::Method;
use serde::Deserialize;
use url::Url;

use super::{GitHubApp, GitHubError};

#[derive(Deserialize)]
struct Repository {
    id: u64,
}

#[derive(Deserialize)]
struct PullReference {
    number: u64,
}

#[derive(Deserialize)]
struct WorkflowRun {
    id: u64,
    #[serde(default)]
    status: String,
    event: String,
    head_sha: String,
    repository: Repository,
    pull_requests: Vec<PullReference>,
}

#[derive(Deserialize)]
struct PullHead {
    sha: String,
    repo: Repository,
}

#[derive(Deserialize)]
struct PullRequest {
    number: u64,
    head: PullHead,
    base: PullHead,
    merge_commit_sha: Option<String>,
}

fn workflow_id(ci_url: &str, repositories: &[&str]) -> Result<u64, GitHubError> {
    let invalid = || {
        GitHubError::Invalid(
            "The eval run must come from the connected GitHub repository's Actions workflow",
        )
    };
    let url = Url::parse(ci_url).map_err(|_| invalid())?;
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    let parts = url
        .path()
        .trim_start_matches('/')
        .split('/')
        .collect::<Vec<_>>();
    if parts.len() != 5
        || !repositories
            .iter()
            .any(|repository| format!("{}/{}", parts[0], parts[1]).eq_ignore_ascii_case(repository))
        || parts[2..4] != ["actions", "runs"]
    {
        return Err(invalid());
    }
    parts[4]
        .parse::<u64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(invalid)
}

pub(super) struct WorkflowTarget {
    pub id: u64,
    pub path: String,
}

async fn workflow_target(
    github: &GitHubApp,
    token: &str,
    connection: &Connection,
    ci_url: &str,
) -> Result<WorkflowTarget, GitHubError> {
    let current = github.repository(connection, token).await?;
    let id = workflow_id(ci_url, &[&connection.repository, &current.full_name])?;
    Ok(WorkflowTarget {
        id,
        path: format!("/repos/{}", current.full_name),
    })
}

fn validate_workflow(
    run: &lens_contract::eval::EvalRun,
    connection: &Connection,
    id: u64,
    workflow: &WorkflowRun,
    pull: Option<&PullRequest>,
) -> Result<(), GitHubError> {
    let invalid = || {
        GitHubError::Invalid(
            "The eval commit and pull request do not match the connected GitHub Actions run",
        )
    };
    if workflow.id != id
        || workflow.repository.id != connection.repository_id
        || run.version.len() != 40
        || !run.version.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(invalid());
    }
    match (run.pr, pull) {
        (Some(number), Some(pull))
            if workflow.event == "pull_request"
                && workflow.pull_requests.iter().any(|pr| pr.number == number)
                && pull.number == number
                && pull.head.repo.id == connection.repository_id
                && pull.base.repo.id == connection.repository_id
                && (pull.head.sha == workflow.head_sha
                    || pull.merge_commit_sha.as_ref() == Some(&workflow.head_sha))
                && (run.version == pull.head.sha
                    || pull.merge_commit_sha.as_ref() == Some(&run.version)) =>
        {
            Ok(())
        }
        (None, None)
            if matches!(workflow.event.as_str(), "push" | "workflow_dispatch")
                && run.version == workflow.head_sha =>
        {
            Ok(())
        }
        _ => Err(invalid()),
    }
}

pub(super) struct VerifiedWorkflow {
    pub target: WorkflowTarget,
    pub status: String,
}

pub(super) async fn verify(
    github: &GitHubApp,
    token: &str,
    connection: &Connection,
    run: &lens_contract::eval::EvalRun,
    ci_url: &str,
) -> Result<VerifiedWorkflow, GitHubError> {
    let target = workflow_target(github, token, connection, ci_url).await?;
    let workflow: WorkflowRun = github
        .request(
            Method::GET,
            &format!("{}/actions/runs/{}", target.path, target.id),
            token,
            None,
        )
        .await?;
    let pull = match run.pr {
        Some(number) => Some(
            github
                .request(
                    Method::GET,
                    &format!("{}/pulls/{number}", target.path),
                    token,
                    None,
                )
                .await?,
        ),
        None => None,
    };
    validate_workflow(run, connection, target.id, &workflow, pull.as_ref())?;
    Ok(VerifiedWorkflow {
        target,
        status: workflow.status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use rstest::{fixture, rstest};
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
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
    fn run() -> lens_contract::eval::EvalRun {
        let mut run: lens_contract::eval::EvalRun = serde_json::from_str(include_str!(
            "../../../../../sdk/tests/fixtures/lens_eval/eval_run_no_baseline.json"
        ))
        .unwrap();
        run.version = "a".repeat(40);
        run.pr = Some(7);
        run
    }

    #[rstest]
    #[case::valid("https://github.com/Org/Repo/actions/runs/123", Some(123))]
    #[case::foreign("https://github.com/org/other/actions/runs/123", None)]
    #[case::fake_host("https://github.com.example/org/repo/actions/runs/123", None)]
    #[case::credentials("https://user@github.com/org/repo/actions/runs/123", None)]
    #[case::query("https://github.com/org/repo/actions/runs/123?a=b", None)]
    #[case::not_workflow("https://github.com/org/repo/pull/123", None)]
    fn workflow_provenance(#[case] value: &str, #[case] expected: Option<u64>) {
        assert_eq!(workflow_id(value, &["org/repo"]).ok(), expected);
    }

    #[rstest]
    #[case::historic_name("org/repo", 9, true)]
    #[case::current_name("org/renamed", 9, true)]
    #[case::foreign_name("org/other", 9, false)]
    #[case::foreign_id("org/renamed", 8, false)]
    #[tokio::test]
    async fn resolves_renamed_repositories(
        connection: Connection,
        #[case] url_repository: &str,
        #[case] repository_id: u64,
        #[case] accepted: bool,
    ) {
        let server = MockServer::start().await;
        let github = GitHubApp::new(
            "lens-test".into(),
            "client".into(),
            "secret".into(),
            include_str!("../../tests/fixtures/github-test-key.pem"),
            Url::parse("https://lens.example").unwrap(),
        )
        .unwrap()
        .with_endpoints(&server.uri());
        Mock::given(method("GET")).and(path("/installation/repositories"))
            .and(wiremock::matchers::header("Authorization", "Bearer scoped-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"repositories": [{"id": repository_id, "full_name": "org/renamed", "default_branch": "main"}]})))
            .expect(1).mount(&server).await;
        let target = workflow_target(
            &github,
            "scoped-token",
            &connection,
            &format!("https://github.com/{url_repository}/actions/runs/123"),
        )
        .await;
        assert_eq!(target.is_ok(), accepted);
        if let Ok(target) = target {
            assert_eq!(target.id, 123);
            assert_eq!(target.path, "/repos/org/renamed");
        }
    }

    #[rstest]
    #[case::head_commit('a', 'a', 9, 9, 7, true)]
    #[case::merge_commit('b', 'a', 9, 9, 7, true)]
    #[case::merge_workflow('b', 'b', 9, 9, 7, true)]
    #[case::foreign_commit('c', 'a', 9, 9, 7, false)]
    #[case::foreign_repository('a', 'a', 8, 9, 7, false)]
    #[case::fork('a', 'a', 9, 8, 7, false)]
    #[case::unrelated_pr('a', 'a', 9, 9, 8, false)]
    #[case::stale_workflow('b', 'c', 9, 9, 7, false)]
    fn verifies_pr_commit(
        mut run: lens_contract::eval::EvalRun,
        #[case] version: char,
        #[case] workflow_sha: char,
        #[case] repository_id: u64,
        #[case] head_repository_id: u64,
        #[case] number: u64,
        #[case] accepted: bool,
    ) {
        run.version = version.to_string().repeat(40);
        let workflow = WorkflowRun {
            id: 123,
            status: String::new(),
            event: "pull_request".into(),
            head_sha: workflow_sha.to_string().repeat(40),
            repository: Repository { id: repository_id },
            pull_requests: vec![PullReference { number }],
        };
        let pull = PullRequest {
            number: 7,
            head: PullHead {
                sha: "a".repeat(40),
                repo: Repository {
                    id: head_repository_id,
                },
            },
            base: PullHead {
                sha: "d".repeat(40),
                repo: Repository { id: 9 },
            },
            merge_commit_sha: Some("b".repeat(40)),
        };
        assert_eq!(
            validate_workflow(&run, &connection(), 123, &workflow, Some(&pull)).is_ok(),
            accepted
        );
    }

    #[rstest]
    #[case::push("push", true)]
    #[case::manual("workflow_dispatch", true)]
    #[case::pr_without_number("pull_request", false)]
    fn verifies_baseline(
        connection: Connection,
        mut run: lens_contract::eval::EvalRun,
        #[case] event: &str,
        #[case] accepted: bool,
    ) {
        run.pr = None;
        let workflow = WorkflowRun {
            id: 123,
            status: String::new(),
            event: event.into(),
            head_sha: run.version.clone(),
            repository: Repository { id: 9 },
            pull_requests: vec![],
        };
        assert_eq!(
            validate_workflow(&run, &connection, 123, &workflow, None).is_ok(),
            accepted
        );
    }
}
