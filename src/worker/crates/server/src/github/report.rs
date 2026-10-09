use std::{collections::BTreeSet, sync::Arc, time::Duration};

use axum::{Json, extract::State, http::HeaderMap};
use chrono::{DateTime, Utc};
use lens_auth::SessionRepository;
use lens_contract::{eval::RunStatus, github::Connection};
use lens_evals_sdk::{model::Report, reporting};
use litellm_storage_clickhouse::{
    Error as StorageError, EvalError,
    evals::{EvalStore, StoredRun},
    state::{Change, ClickHouseState},
};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use url::Url;

use super::{App, GitHubApp, GitHubError, owner};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    run_ids: Vec<String>,
}

impl Request {
    fn validate(&self) -> Result<(), GitHubError> {
        if self.run_ids.is_empty()
            || self
                .run_ids
                .iter()
                .any(|id| id.is_empty() || id.len() > 200)
            || self.run_ids.iter().collect::<BTreeSet<_>>().len() != self.run_ids.len()
        {
            return Err(GitHubError::Invalid(
                "Provide distinct, nonempty Lens run IDs",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Deserialize, Serialize)]
pub(super) struct Published {
    run_id: String,
    comment_url: Option<String>,
    check_url: String,
}

#[derive(Serialize)]
pub(super) struct Response {
    reports: Vec<Published>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReportInput {
    pub run: lens_contract::eval::EvalRun,
    pub baseline: Option<lens_contract::eval::EvalRun>,
    pub ci_url: String,
}

impl ReportInput {
    pub(super) fn bind_urls(&mut self, origin: &str) -> Result<(), GitHubError> {
        let base = Url::parse(origin).map_err(|_| GitHubError::Credentials)?;
        let link = |eval: &str, id: &str, case: Option<&str>| -> String {
            let mut url = base.join("/ui/").expect("validated origin");
            url.query_pairs_mut()
                .append_pair("tab", "evals")
                .append_pair("eval", eval)
                .append_pair("eval_run", id);
            if let Some(case) = case {
                url.query_pairs_mut().append_pair("eval_case", case);
            }
            url.into()
        };
        let bind = |run: &mut lens_contract::eval::EvalRun| -> Result<(), GitHubError> {
            if !crate::evals::valid_eval_name(&run.eval) || run.eval.len() > 200 {
                return Err(GitHubError::Invalid("Provide a valid eval identifier"));
            }
            run.url = link(&run.eval, &run.id, None);
            if let Some(summary) = &mut run.summary {
                for case in summary.regressions.iter_mut().chain(&mut summary.fixed) {
                    case.candidate_url = link(&run.eval, &run.id, Some(&case.case_id));
                    case.baseline_url = summary
                        .baseline_run_id
                        .as_deref()
                        .map(|id| link(&run.eval, id, Some(&case.case_id)))
                        .unwrap_or_default();
                }
            }
            Ok(())
        };
        bind(&mut self.run)?;
        if let Some(baseline) = &mut self.baseline {
            bind(baseline)?;
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
struct Publication {
    token: String,
    lease_until: DateTime<Utc>,
    report: Option<Published>,
}

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

#[derive(Deserialize)]
struct CommentUser {
    login: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct Comment {
    body: Option<String>,
    user: Option<CommentUser>,
    html_url: String,
}

#[derive(Deserialize)]
struct CheckApp {
    slug: String,
}

#[derive(Deserialize)]
struct Check {
    external_id: Option<String>,
    app: Option<CheckApp>,
    html_url: String,
}

#[derive(Deserialize)]
struct Checks {
    check_runs: Vec<Check>,
}

#[derive(Deserialize)]
struct Link {
    html_url: String,
}

pub(super) async fn publish<R: SessionRepository>(
    State(app): State<Arc<App<R>>>,
    headers: HeaderMap,
    Json(request): Json<Request>,
) -> Result<Json<Response>, GitHubError> {
    let identity = crate::auth::identity(&app.authentication, &headers, &Method::POST).await?;
    let owner = owner(&identity, true)?;
    request.validate()?;
    let store = EvalStore::new(app.store.0.clone());
    let mut reports = Vec::new();
    for id in request.run_ids {
        let stored = store.get(&owner.scope, &id).await.map_err(eval_error)?;
        if stored.run.status != RunStatus::Done || stored.run.summary.is_none() {
            return Err(GitHubError::Invalid(
                "Only completed eval runs can be reported",
            ));
        }
        let connection = app
            .store
            .connection(&owner.scope, &stored.run.agent)
            .await?
            .ok_or(GitHubError::Invalid(
                "Connect this agent to a GitHub repository first",
            ))?;
        let input = stored_input(&store, &stored).await?;
        let report = match (&app.settings.remote, &app.settings.github) {
            (Some(remote), _) => {
                super::remote::cleanup_retired(&app, remote, &owner.scope, &stored.run.agent)
                    .await?;
                let credentials = super::remote::credentials(&app, &owner.scope, &stored.run.agent)
                    .await?
                    .ok_or(GitHubError::Credentials)?;
                remote.report(&credentials, input).await?
            }
            (_, Some(github)) => {
                publish_input(github, &app.store.0, &stored.team, &connection, input).await?
            }
            _ => return Err(GitHubError::NotConfigured),
        };
        reports.push(report);
    }
    Ok(Json(Response { reports }))
}

fn eval_error(error: EvalError) -> GitHubError {
    match error {
        EvalError::Storage(error) => error.into(),
        _ => GitHubError::Invalid("The eval run is not available to this Lens credential"),
    }
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

struct WorkflowTarget {
    id: u64,
    path: String,
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

async fn stored_input(store: &EvalStore, stored: &StoredRun) -> Result<ReportInput, GitHubError> {
    let baseline = match stored
        .run
        .summary
        .as_ref()
        .and_then(|summary| summary.baseline_run_id.as_deref())
    {
        Some(id) => Some(store.get(&stored.team, id).await.map_err(eval_error)?.run),
        None => None,
    };
    Ok(ReportInput {
        run: stored.run.clone(),
        baseline,
        ci_url: stored.request.ci_url.clone(),
    })
}

pub(super) async fn publish_input(
    github: &GitHubApp,
    state: &ClickHouseState,
    namespace: &str,
    connection: &Connection,
    input: ReportInput,
) -> Result<Published, GitHubError> {
    if input.run.status != RunStatus::Done
        || input.run.summary.is_none()
        || input.run.agent != connection.agent
        || input.run.id.is_empty()
        || input.run.id.len() > 200
        || !crate::evals::valid_eval_name(&input.run.eval)
        || input.run.eval.len() > 200
        || input.ci_url.len() > 2048
    {
        return Err(GitHubError::Invalid(
            "Provide a completed eval run for the connected agent",
        ));
    }
    if input.baseline.as_ref().is_some_and(|baseline| {
        baseline.agent != input.run.agent
            || baseline.eval != input.run.eval
            || input
                .run
                .summary
                .as_ref()
                .and_then(|summary| summary.baseline_run_id.as_ref())
                != Some(&baseline.id)
    }) {
        return Err(GitHubError::Invalid(
            "The eval baseline does not match this report",
        ));
    }
    let key = format!(
        "github-report/{:x}",
        Sha256::digest(format!(
            "{}\0{}\0{}",
            namespace, input.run.id, connection.repository_id
        ))
    );
    let previous = state.read(&key).await?;
    if !previous.value.is_null() {
        let publication: Publication = serde_json::from_value(previous.value.clone())
            .map_err(|_| StorageError::InvalidState)?;
        if let Some(report) = publication.report {
            return Ok(report);
        }
        if publication.lease_until > Utc::now() {
            return Err(StorageError::StateConflict.into());
        }
    }
    let claim = Publication {
        token: uuid::Uuid::new_v4().to_string(),
        lease_until: Utc::now() + chrono::TimeDelta::seconds(60),
        report: None,
    };
    let value = serde_json::to_value(&claim).map_err(|_| StorageError::InvalidState)?;
    state.commit(vec![Change { previous, value }]).await?;
    let claimed = state.read(&key).await?;
    let active: Publication =
        serde_json::from_value(claimed.value.clone()).map_err(|_| StorageError::InvalidState)?;
    if active.token != claim.token
        || active.lease_until <= Utc::now() + chrono::TimeDelta::seconds(30)
    {
        return Err(StorageError::StateConflict.into());
    }
    let report = tokio::time::timeout(Duration::from_secs(25), async {
        let token = github.installation_token(connection).await?;
        let WorkflowTarget { id, path } =
            workflow_target(github, &token, connection, &input.ci_url).await?;
        let workflow: WorkflowRun = github
            .request(
                Method::GET,
                &format!("{path}/actions/runs/{id}"),
                &token,
                None,
            )
            .await?;
        let pull = match input.run.pr {
            Some(number) => Some(
                github
                    .request(Method::GET, &format!("{path}/pulls/{number}"), &token, None)
                    .await?,
            ),
            None => None,
        };
        validate_workflow(&input.run, connection, id, &workflow, pull.as_ref())?;
        let report = Report {
            run: sdk_run(input.run)?,
            baseline: input.baseline.map(sdk_run).transpose()?,
            trials: Vec::new(),
        };
        publish_report(github, &token, &path, &key, &report).await
    })
    .await
    .map_err(|_| {
        GitHubError::Invalid("GitHub reporting timed out. Retry this run after one minute")
    })??;
    state
        .commit(vec![Change {
            previous: claimed,
            value: serde_json::to_value(Publication {
                report: Some(report.clone()),
                ..claim
            })
            .map_err(|_| StorageError::InvalidState)?,
        }])
        .await?;
    Ok(report)
}

fn sdk_run(
    run: lens_contract::eval::EvalRun,
) -> Result<lens_evals_sdk::model::EvalRun, GitHubError> {
    serde_json::to_value(run)
        .and_then(serde_json::from_value)
        .map_err(|_| StorageError::InvalidState.into())
}

async fn publish_report(
    github: &GitHubApp,
    token: &str,
    path: &str,
    identity: &str,
    report: &Report,
) -> Result<Published, GitHubError> {
    let marker = format!("<!-- {identity} -->");
    let body = format!(
        "{marker}\n{}",
        reporting::markdown(report).map_err(|_| StorageError::InvalidState)?
    );
    let body = if body.chars().count() > 60000 {
        format!(
            "{}\n\n{}",
            body.chars().take(59000).collect::<String>(),
            reporting::link("View full report in Lens", &report.run.url)
        )
    } else {
        body
    };
    let comment_url = match report.run.pr {
        Some(pr) => Some(publish_comment(github, token, path, pr, &marker, &body).await?),
        None => None,
    };
    let check_url = publish_check(github, token, path, identity, report, &body).await?;
    Ok(Published {
        run_id: report.run.id.clone(),
        comment_url,
        check_url,
    })
}

async fn publish_comment(
    github: &GitHubApp,
    token: &str,
    path: &str,
    pr: u64,
    marker: &str,
    body: &str,
) -> Result<String, GitHubError> {
    let bot = format!("{}[bot]", github.slug);
    for page in 1..=10 {
        let comments: Vec<Comment> = github
            .request(
                Method::GET,
                &format!("{path}/issues/{pr}/comments?per_page=100&page={page}"),
                token,
                None,
            )
            .await?;
        if let Some(comment) = comments.iter().find(|comment| {
            comment
                .user
                .as_ref()
                .is_some_and(|user| user.kind == "Bot" && user.login == bot)
                && comment
                    .body
                    .as_deref()
                    .is_some_and(|body| body.starts_with(marker))
        }) {
            return Ok(comment.html_url.clone());
        }
        if comments.len() < 100 {
            let result: Link = github
                .request(
                    Method::POST,
                    &format!("{path}/issues/{pr}/comments"),
                    token,
                    Some(json!({"body": body})),
                )
                .await?;
            return Ok(result.html_url);
        }
    }
    Err(GitHubError::Invalid(
        "Too many PR comments to safely reconcile this report",
    ))
}

async fn publish_check(
    github: &GitHubApp,
    token: &str,
    path: &str,
    identity: &str,
    report: &Report,
    body: &str,
) -> Result<String, GitHubError> {
    for page in 1..=10 {
        let checks: Checks = github
            .request(
                Method::GET,
                &format!(
                    "{path}/commits/{}/check-runs?filter=all&per_page=100&page={page}",
                    report.run.version
                ),
                token,
                None,
            )
            .await?;
        if let Some(check) = checks.check_runs.iter().find(|check| {
            check
                .app
                .as_ref()
                .is_some_and(|app| app.slug == github.slug)
                && check.external_id.as_deref() == Some(identity)
        }) {
            return Ok(check.html_url.clone());
        }
        if checks.check_runs.len() < 100 {
            let result: Link = github.request(Method::POST, &format!("{path}/check-runs"), token, Some(json!({
                "name": format!("Lens / {}", report.run.eval), "head_sha": report.run.version,
                "external_id": identity, "status": "completed", "conclusion": reporting::conclusion(report).map_err(|_| StorageError::InvalidState)?,
                "details_url": report.run.url, "output": { "title": format!("Lens / {}", report.run.eval), "summary": body }
            }))).await?;
            return Ok(result.html_url);
        }
    }
    Err(GitHubError::Invalid(
        "Too many checks to safely reconcile this report",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::{fixture, rstest};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_partial_json, method, path},
    };

    #[rstest]
    #[case::large_batch((0..11).map(|index| format!("run-{index}")).collect(), true)]
    #[case::empty(vec![], false)]
    #[case::empty_id(vec![String::new()], false)]
    #[case::duplicate(vec!["run".into(), "run".into()], false)]
    #[case::invalid_id(vec!["x".repeat(201)], false)]
    fn report_batches_validate_identifiers_without_a_count_ceiling(
        #[case] run_ids: Vec<String>,
        #[case] accepted: bool,
    ) {
        assert_eq!(Request { run_ids }.validate().is_ok(), accepted);
    }

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
    fn report_links_are_rebuilt_from_confirmed_origin(mut run: lens_contract::eval::EvalRun) {
        let summary = run.summary.as_mut().unwrap();
        summary.baseline_run_id = Some("previous".into());
        summary.regressions = vec![lens_contract::eval::CaseDiff {
            case_id: "case&redirect=elsewhere".into(),
            title: "Case".into(),
            critical: false,
            baseline_url: "https://attacker.example/baseline".into(),
            candidate_url: "https://attacker.example/candidate".into(),
        }];
        summary.fixed = summary.regressions.clone();
        let mut input = ReportInput {
            baseline: Some(run.clone()),
            run,
            ci_url: String::new(),
        };
        input.bind_urls("https://confirmed.example").unwrap();
        let serialized = serde_json::to_string(&input).unwrap();
        assert!(!serialized.contains("attacker.example"));
        assert!(!serialized.contains("localhost:8765"));
        let summary = input.run.summary.unwrap();
        let candidate = Url::parse(&summary.regressions[0].candidate_url).unwrap();
        assert_eq!(
            candidate.origin().ascii_serialization(),
            "https://confirmed.example"
        );
        assert_eq!(
            candidate
                .query_pairs()
                .find(|(key, _)| key == "eval_case")
                .unwrap()
                .1,
            "case&redirect=elsewhere"
        );
        assert!(
            summary.regressions[0]
                .baseline_url
                .contains("eval_run=previous")
        );
    }

    #[rstest]
    #[case::markdown("demo\n[click](https://attacker.example)")]
    #[case::html("demo --> <img src='https://attacker.example'>")]
    #[case::empty("")]
    fn report_rejects_eval_markup(mut run: lens_contract::eval::EvalRun, #[case] name: &str) {
        run.eval = name.into();
        let mut input = ReportInput {
            run,
            baseline: None,
            ci_url: String::new(),
        };
        assert!(matches!(
            input.bind_urls("https://confirmed.example"),
            Err(GitHubError::Invalid(_))
        ));
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

    #[rstest]
    #[case::owned("lens-test[bot]", "lens-test", 0)]
    #[case::foreign("somebody", "another-app", 1)]
    #[tokio::test]
    async fn reconciles_only_app_owned_reports(
        run: lens_contract::eval::EvalRun,
        #[case] login: &str,
        #[case] slug: &str,
        #[case] creates: u64,
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
        let identity = "github-report/test";
        Mock::given(method("GET")).and(path("/repos/org/repo/issues/7/comments"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"body": "<!-- github-report/test --> previous", "user": {"login": login, "type": "Bot"}, "html_url": "https://github.com/org/repo/pull/7#comment"}])))
            .expect(1).mount(&server).await;
        Mock::given(method("GET")).and(path(format!("/repos/org/repo/commits/{}/check-runs", run.version)))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"check_runs": [{"external_id": identity, "app": {"slug": slug}, "html_url": "https://github.com/org/repo/check"}]})))
            .expect(1).mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/repos/org/repo/issues/7/comments"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(
                    json!({"html_url": "https://github.com/org/repo/pull/7#comment"}),
                ),
            )
            .expect(creates)
            .mount(&server)
            .await;
        Mock::given(method("POST")).and(path("/repos/org/repo/check-runs"))
            .and(body_partial_json(json!({"external_id": identity, "head_sha": run.version, "status": "completed", "conclusion": "neutral"})))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"html_url": "https://github.com/org/repo/check"})))
            .expect(creates).mount(&server).await;
        let result = publish_report(
            &github,
            "installation-token",
            "/repos/org/repo",
            identity,
            &Report {
                run: sdk_run(run).unwrap(),
                baseline: None,
                trials: vec![],
            },
        )
        .await
        .unwrap();
        assert_eq!(
            result.comment_url.as_deref(),
            Some("https://github.com/org/repo/pull/7#comment")
        );
        assert_eq!(result.check_url, "https://github.com/org/repo/check");
    }
}
