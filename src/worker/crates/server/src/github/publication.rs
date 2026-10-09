use std::time::Duration;

use chrono::{DateTime, Utc};
use lens_contract::github::{Connection, ProgressState};
use lens_evals_sdk::{model::Report, reporting};
use litellm_storage_clickhouse::{
    Error as StorageError,
    state::{Change, ClickHouseState},
};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::{
    GitHubApp, GitHubError,
    progress::ProgressInput,
    report::{Published, WorkflowRun, WorkflowTarget, validate_workflow, workflow_target},
};

pub(super) enum Content {
    Finished(Box<Report>),
    Progress(ProgressInput),
}

struct Context<'a> {
    name: &'a str,
    version: &'a str,
    pr: Option<u64>,
    ci_url: &'a str,
    run_id: &'a str,
    details_url: &'a str,
}

impl Content {
    fn context(&self) -> Context<'_> {
        match self {
            Self::Finished(report) => Context {
                name: &report.run.eval,
                version: &report.run.version,
                pr: report.run.pr,
                ci_url: &report.run.ci_url,
                run_id: &report.run.id,
                details_url: &report.run.url,
            },
            Self::Progress(input) => Context {
                name: &input.request.name,
                version: &input.request.version,
                pr: Some(input.request.pr),
                ci_url: &input.request.ci_url,
                run_id: "",
                details_url: &input.request.ci_url,
            },
        }
    }

    fn phase(&self) -> Phase {
        match self {
            Self::Finished(_) => Phase::Complete,
            Self::Progress(input) => match input.request.state {
                ProgressState::Running => Phase::Running,
                ProgressState::Failed => Phase::Failed,
            },
        }
    }

    fn markdown(&self) -> Result<String, GitHubError> {
        match self {
            Self::Finished(report) => {
                reporting::markdown(report).map_err(|_| StorageError::InvalidState.into())
            }
            Self::Progress(input) => Ok(reporting::progress(&input.request)),
        }
    }

    fn conclusion(&self) -> Result<Option<&'static str>, GitHubError> {
        match self {
            Self::Finished(report) => reporting::conclusion(report)
                .map(Some)
                .map_err(|_| StorageError::InvalidState.into()),
            Self::Progress(input) => Ok(match input.request.state {
                ProgressState::Running => None,
                ProgressState::Failed => Some("failure"),
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
enum Phase {
    Running,
    Failed,
    Complete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct Version {
    workflow: u64,
    attempt: u64,
    phase: Phase,
}

#[derive(Serialize, Deserialize)]
struct Publication {
    token: String,
    lease_until: DateTime<Utc>,
    version: Version,
    report: Option<Published>,
}

impl Publication {
    fn existing(
        &self,
        version: Version,
        content: &Content,
    ) -> Result<Option<Published>, GitHubError> {
        if self.lease_until > Utc::now() && self.report.is_none() {
            return Err(StorageError::StateConflict.into());
        }
        if self.version < version {
            return Ok(None);
        }
        if self.version.workflow != version.workflow || self.version.attempt != version.attempt {
            return Err(GitHubError::Invalid(
                "A newer workflow already published this evaluation",
            ));
        }
        match (&self.report, content) {
            (Some(report), Content::Progress(_)) => Ok(Some(report.clone())),
            (Some(report), Content::Finished(_)) if report.run_id == content.context().run_id => {
                Ok(Some(report.clone()))
            }
            (Some(_), _) => Err(GitHubError::Invalid(
                "This workflow already reported another completed run",
            )),
            (None, _) => Ok(None),
        }
    }
}

fn identity(namespace: &str, connection: &Connection, context: &Context<'_>) -> String {
    let target = context
        .pr
        .map(|pr| format!("pr/{pr}"))
        .unwrap_or_else(|| format!("run/{}", context.run_id));
    format!(
        "github-publication/{:x}",
        Sha256::digest(format!(
            "{namespace}\0{}\0{target}\0{}",
            connection.repository_id, context.name
        ))
    )
}

pub(super) async fn publish(
    github: &GitHubApp,
    state: &ClickHouseState,
    namespace: &str,
    connection: &Connection,
    content: Content,
) -> Result<Published, GitHubError> {
    tokio::time::timeout(
        Duration::from_secs(25),
        publish_inner(github, state, namespace, connection, content),
    )
    .await
    .map_err(|_| GitHubError::Invalid("GitHub reporting timed out. Retry after one minute"))?
}

async fn publish_inner(
    github: &GitHubApp,
    state: &ClickHouseState,
    namespace: &str,
    connection: &Connection,
    content: Content,
) -> Result<Published, GitHubError> {
    let context = content.context();
    let token = github.installation_token(connection).await?;
    let WorkflowTarget { id, path } =
        workflow_target(github, &token, connection, context.ci_url).await?;
    let workflow: WorkflowRun = github
        .request(
            Method::GET,
            &format!("{path}/actions/runs/{id}"),
            &token,
            None,
        )
        .await?;
    let pull = match context.pr {
        Some(number) => Some(
            github
                .request(Method::GET, &format!("{path}/pulls/{number}"), &token, None)
                .await?,
        ),
        None => None,
    };
    validate_workflow(
        context.version,
        context.pr,
        connection,
        id,
        &workflow,
        pull.as_ref(),
    )?;
    let version = Version {
        workflow: id,
        attempt: workflow.run_attempt,
        phase: content.phase(),
    };
    let key = identity(namespace, connection, &context);
    let previous = state.read(&key).await?;
    if !previous.value.is_null() {
        let publication: Publication = serde_json::from_value(previous.value.clone())
            .map_err(|_| StorageError::InvalidState)?;
        if let Some(report) = publication.existing(version, &content)? {
            return Ok(report);
        }
    }
    let claim = Publication {
        token: uuid::Uuid::new_v4().to_string(),
        lease_until: Utc::now() + chrono::TimeDelta::seconds(60),
        version,
        report: None,
    };
    state
        .commit(vec![Change {
            previous,
            value: serde_json::to_value(&claim).map_err(|_| StorageError::InvalidState)?,
        }])
        .await?;
    let claimed = state.read(&key).await?;
    let active: Publication =
        serde_json::from_value(claimed.value.clone()).map_err(|_| StorageError::InvalidState)?;
    if active.token != claim.token
        || active.lease_until <= Utc::now() + chrono::TimeDelta::seconds(30)
    {
        return Err(StorageError::StateConflict.into());
    }
    let app: AppIdentity = github
        .request(Method::GET, &format!("/apps/{}", github.slug), &token, None)
        .await?;
    if app.slug != github.slug || app.id == 0 {
        return Err(GitHubError::Credentials);
    }
    let report = publish_report(github, &token, &path, &key, &app, &content).await?;
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

#[derive(Deserialize)]
struct AppIdentity {
    id: u64,
    slug: String,
}

#[derive(Deserialize)]
struct CommentUser {
    login: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct Comment {
    id: u64,
    body: Option<String>,
    user: Option<CommentUser>,
    performed_via_github_app: Option<AppIdentity>,
}

#[derive(Deserialize)]
struct Check {
    id: u64,
    external_id: Option<String>,
    app: Option<AppIdentity>,
}

#[derive(Deserialize)]
struct Checks {
    check_runs: Vec<Check>,
}

#[derive(Deserialize)]
struct Link {
    html_url: String,
}

async fn publish_report(
    github: &GitHubApp,
    token: &str,
    path: &str,
    identity: &str,
    app: &AppIdentity,
    content: &Content,
) -> Result<Published, GitHubError> {
    let context = content.context();
    let marker = format!("<!-- {identity} -->");
    let body = format!("{marker}\n{}", content.markdown()?);
    let body = if body.chars().count() > 60000 {
        format!(
            "{}\n\n{}",
            body.chars().take(59000).collect::<String>(),
            reporting::link("View full report in Lens", context.details_url)
        )
    } else {
        body
    };
    let comment_url = match context.pr {
        Some(pr) => Some(publish_comment(github, token, path, pr, app, &marker, &body).await?),
        None => None,
    };
    let check_identity = format!("{identity}/{}", context.version);
    let check_url =
        publish_check(github, token, path, &check_identity, app, content, &body).await?;
    Ok(Published {
        run_id: context.run_id.into(),
        comment_url,
        check_url,
    })
}

async fn publish_comment(
    github: &GitHubApp,
    token: &str,
    path: &str,
    pr: u64,
    app: &AppIdentity,
    marker: &str,
    body: &str,
) -> Result<String, GitHubError> {
    let bot = format!("{}[bot]", app.slug);
    for page in 1..=10 {
        let comments: Vec<Comment> = github
            .request(
                Method::GET,
                &format!("{path}/issues/{pr}/comments?per_page=100&page={page}"),
                token,
                None,
            )
            .await?;
        let owned = comments.iter().find(|comment| {
            comment
                .user
                .as_ref()
                .is_some_and(|user| user.kind == "Bot" && user.login == bot)
                && comment
                    .performed_via_github_app
                    .as_ref()
                    .is_some_and(|owner| owner.id == app.id && owner.slug == app.slug)
                && comment
                    .body
                    .as_ref()
                    .is_some_and(|body| body.starts_with(marker))
        });
        if let Some(comment) = owned {
            let result: Link = github
                .request(
                    Method::PATCH,
                    &format!("{path}/issues/comments/{}", comment.id),
                    token,
                    Some(json!({"body":body})),
                )
                .await?;
            return Ok(result.html_url);
        }
        if comments.len() < 100 {
            let result: Link = github
                .request(
                    Method::POST,
                    &format!("{path}/issues/{pr}/comments"),
                    token,
                    Some(json!({"body":body})),
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
    app: &AppIdentity,
    content: &Content,
    body: &str,
) -> Result<String, GitHubError> {
    let context = content.context();
    let conclusion = content.conclusion()?;
    let payload = json!({"name": format!("Lens / {}", context.name), "external_id": identity,
        "status": if conclusion.is_some() { "completed" } else { "in_progress" },
        "details_url": context.details_url, "output": {"title": format!("Lens / {}", context.name), "summary": body}});
    let mut payload = payload;
    if let Some(conclusion) = conclusion {
        payload["conclusion"] = json!(conclusion);
    }
    for page in 1..=10 {
        let checks: Checks = github
            .request(
                Method::GET,
                &format!(
                    "{path}/commits/{}/check-runs?filter=all&per_page=100&page={page}",
                    context.version
                ),
                token,
                None,
            )
            .await?;
        if let Some(check) = checks.check_runs.iter().find(|check| {
            check
                .app
                .as_ref()
                .is_some_and(|owner| owner.id == app.id && owner.slug == app.slug)
                && check.external_id.as_deref() == Some(identity)
        }) {
            let result: Link = github
                .request(
                    Method::PATCH,
                    &format!("{path}/check-runs/{}", check.id),
                    token,
                    Some(payload),
                )
                .await?;
            return Ok(result.html_url);
        }
        if checks.check_runs.len() < 100 {
            payload["head_sha"] = json!(context.version);
            let result: Link = github
                .request(
                    Method::POST,
                    &format!("{path}/check-runs"),
                    token,
                    Some(payload),
                )
                .await?;
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
    use lens_contract::github::ProgressRequest;
    use rstest::rstest;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_partial_json, method, path},
    };

    fn progress() -> Content {
        Content::Progress(ProgressInput {
            agent: "agent".into(),
            request: ProgressRequest {
                name: "demo".into(),
                version: "a".repeat(40),
                pr: 7,
                ci_url: "https://github.com/org/repo/actions/runs/50".into(),
                state: ProgressState::Running,
            },
        })
    }

    fn finished() -> Content {
        let run = serde_json::from_str(include_str!(
            "../../../../../sdk/tests/fixtures/lens_eval/eval_run_no_baseline.json"
        ))
        .unwrap();
        Content::Finished(Box::new(Report {
            run,
            baseline: None,
            trials: Vec::new(),
        }))
    }

    #[rstest]
    #[case(50, 1, Phase::Running, true)]
    #[case(50, 1, Phase::Failed, true)]
    #[case(50, 2, Phase::Running, false)]
    #[case(51, 1, Phase::Running, false)]
    fn finished_result_survives_late_progress_but_allows_new_runs(
        #[case] workflow: u64,
        #[case] attempt: u64,
        #[case] phase: Phase,
        #[case] cached: bool,
    ) {
        let publication = Publication {
            token: "claim".into(),
            lease_until: Utc::now(),
            version: Version {
                workflow: 50,
                attempt: 1,
                phase: Phase::Complete,
            },
            report: Some(Published {
                run_id: "run".into(),
                comment_url: Some("comment".into()),
                check_url: "check".into(),
            }),
        };
        assert_eq!(
            publication
                .existing(
                    Version {
                        workflow,
                        attempt,
                        phase
                    },
                    &progress()
                )
                .unwrap()
                .is_some(),
            cached
        );
        assert!(
            publication
                .existing(
                    Version {
                        workflow: 49,
                        attempt: 1,
                        phase: Phase::Complete
                    },
                    &finished()
                )
                .is_err()
        );
    }

    #[test]
    fn active_publication_lease_prevents_competing_writes() {
        let version = Version {
            workflow: 50,
            attempt: 1,
            phase: Phase::Running,
        };
        let publication = Publication {
            token: "claim".into(),
            lease_until: Utc::now() + chrono::TimeDelta::seconds(30),
            version,
            report: None,
        };
        assert!(matches!(
            publication.existing(version, &progress()),
            Err(GitHubError::Storage(StorageError::StateConflict))
        ));
    }

    #[rstest]
    #[case::owned("lens-test[bot]", "lens-test", 99, "PATCH")]
    #[case::actions("github-actions[bot]", "github-actions", 1, "POST")]
    #[case::foreign_id("lens-test[bot]", "lens-test", 7, "POST")]
    #[case::foreign_user("somebody", "lens-test", 99, "POST")]
    #[tokio::test]
    async fn updates_only_comments_and_checks_owned_by_the_configured_app(
        #[case] login: &str,
        #[case] slug: &str,
        #[case] app_id: u64,
        #[case] verb: &str,
    ) {
        let server = MockServer::start().await;
        let github = GitHubApp::new(
            "lens-test".into(),
            "client".into(),
            "secret".into(),
            include_str!("../../tests/fixtures/github-test-key.pem"),
            "https://lens.example".parse().unwrap(),
        )
        .unwrap()
        .with_endpoints(&server.uri());
        let app = AppIdentity {
            id: 99,
            slug: "lens-test".into(),
        };
        let identity = "github-publication/test";
        Mock::given(method("GET")).and(path("/repos/org/repo/issues/7/comments"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"id":3,"body":"<!-- github-publication/test --> previous","user":{"login":login,"type":"Bot"},"performed_via_github_app":{"id":app_id,"slug":slug}}])))
            .expect(1).mount(&server).await;
        let context = progress();
        let sha = context.context().version.to_owned();
        let check_identity = format!("{identity}/{sha}");
        Mock::given(method("GET")).and(path(format!("/repos/org/repo/commits/{sha}/check-runs")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"check_runs":[{"id":4,"external_id":check_identity,"app":{"id":app_id,"slug":slug}}]})))
            .expect(1).mount(&server).await;
        let comment_path = if verb == "PATCH" {
            "/repos/org/repo/issues/comments/3"
        } else {
            "/repos/org/repo/issues/7/comments"
        };
        Mock::given(method(verb))
            .and(path(comment_path))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"html_url":"https://github.com/org/repo/pull/7#comment"}),
                ),
            )
            .expect(1)
            .mount(&server)
            .await;
        let check_verb = if app_id == 99 && slug == "lens-test" {
            "PATCH"
        } else {
            "POST"
        };
        let check_path = if check_verb == "PATCH" {
            "/repos/org/repo/check-runs/4"
        } else {
            "/repos/org/repo/check-runs"
        };
        Mock::given(method(check_verb))
            .and(path(check_path))
            .and(body_partial_json(
                json!({"status":"in_progress","external_id":check_identity}),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"html_url":"https://github.com/org/repo/check"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let result = publish_report(
            &github,
            "token",
            "/repos/org/repo",
            identity,
            &app,
            &context,
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
