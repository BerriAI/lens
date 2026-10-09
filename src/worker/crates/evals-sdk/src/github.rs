use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    time::Duration,
};

use lens_contract::github::{ProgressPublished, ProgressRequest, ProgressState};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    Error, Result,
    client::{Client, segment},
    model::{EvalRun, Execution, Report},
    reporting,
    setup::{Settings, env_value},
};

#[derive(Deserialize)]
struct User {
    login: String,
}
#[derive(Deserialize)]
struct Comment {
    id: u64,
    body: Option<String>,
    user: User,
}
#[derive(Deserialize)]
struct Runs {
    runs: Vec<EvalRun>,
}

#[derive(Deserialize)]
struct AppReport {
    run_id: String,
}

#[derive(Deserialize)]
struct AppReports {
    reports: Vec<AppReport>,
}

pub struct GitHub {
    http: reqwest::Client,
    base: String,
    repository: String,
}

impl GitHub {
    pub fn new(base: &str, token: &str, repository: &str) -> Result<Self> {
        let parts = repository.split('/').collect::<Vec<_>>();
        if parts.len() != 2 || parts.iter().any(|part| part.is_empty()) {
            return Err(Error::Configuration(
                "Expected GitHub repository owner/name",
            ));
        }
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "Authorization",
            format!("Bearer {token}")
                .parse()
                .map_err(|_| Error::Configuration("Invalid GitHub token header"))?,
        );
        headers.insert(
            "Accept",
            "application/vnd.github+json".parse().expect("fixed header"),
        );
        headers.insert(
            "X-GitHub-Api-Version",
            "2022-11-28".parse().expect("fixed header"),
        );
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .user_agent("lens-evals")
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(Error::Transport)?;
        Ok(Self {
            http,
            base: crate::client::endpoint(base)?,
            repository: parts.into_iter().map(segment).collect::<Vec<_>>().join("/"),
        })
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<reqwest::Response> {
        let request = self.http.request(
            method,
            format!("{}/repos/{}{path}", self.base, self.repository),
        );
        let response = match body {
            Some(body) => request.json(&body),
            None => request,
        }
        .send()
        .await
        .map_err(Error::Transport)?;
        if !response.status().is_success() {
            return Err(Error::Api {
                status: response.status().as_u16(),
                code: "github_report_failed".into(),
            });
        }
        Ok(response)
    }

    pub async fn publish(&self, report: &Report, sha: &str) -> Result<()> {
        let body = reporting::markdown(report)?;
        if let Some(pr) = report.run.pr {
            self.comment(&report.run.eval, pr, &body).await?;
        }
        self.request(Method::POST, "/check-runs", Some(json!({"name":format!("Lens / {}", report.run.eval),"head_sha":sha,"status":"completed","conclusion":reporting::conclusion(report)?,"details_url":report.run.url,"output":{"title":format!("Lens / {}", report.run.eval),"summary":body.chars().take(65000).collect::<String>()}}))).await?;
        Ok(())
    }

    async fn comment(&self, name: &str, pr: u64, body: &str) -> Result<()> {
        let marker = reporting::marker(name);
        for page in 1.. {
            let comments: Vec<Comment> = Client::decode(
                self.request(
                    Method::GET,
                    &format!("/issues/{pr}/comments?per_page=100&page={page}"),
                    None,
                )
                .await?,
            )
            .await?;
            if let Some(comment) = comments.iter().find(|comment| {
                comment
                    .body
                    .as_ref()
                    .is_some_and(|body| body.starts_with(&marker))
                    && comment.user.login == "github-actions[bot]"
            }) {
                self.request(
                    Method::PATCH,
                    &format!("/issues/comments/{}", comment.id),
                    Some(json!({"body":body})),
                )
                .await?;
                return Ok(());
            }
            if comments.len() < 100 {
                self.request(
                    Method::POST,
                    &format!("/issues/{pr}/comments"),
                    Some(json!({"body":body})),
                )
                .await?;
                return Ok(());
            }
        }
        Ok(())
    }

    pub async fn progress(&self, name: &str, context: &Execution, failed: bool) -> Result<()> {
        let pr = context.pr.ok_or(Error::Configuration(
            "A Lens progress comment requires a pull request event or --pr",
        ))?;
        let body = reporting::progress(&ProgressRequest {
            name: name.to_owned(),
            version: context.version.clone(),
            pr,
            ci_url: context.ci_url.clone(),
            state: if failed {
                ProgressState::Failed
            } else {
                ProgressState::Running
            },
        });
        self.comment(name, pr, &body).await?;
        if failed {
            self.request(Method::POST, "/check-runs", Some(json!({"name":format!("Lens / {name}"),"head_sha":context.version,"status":"completed","conclusion":"failure","details_url":context.ci_url,"output":{"title":format!("Lens / {name}"),"summary":body}}))).await?;
        }
        Ok(())
    }
}

fn required(key: &str) -> Result<String> {
    env_value(key).ok_or(Error::Invalid {
        message: "GitHub reporting requires",
        value: key.to_owned(),
    })
}

fn from_environment() -> Result<GitHub> {
    GitHub::new(
        &env_value("GITHUB_API_URL").unwrap_or_else(|| "https://api.github.com".into()),
        &required("GITHUB_TOKEN")?,
        &required("GITHUB_REPOSITORY")?,
    )
}

pub async fn publish_progress(
    root: &Path,
    name: &str,
    pr: Option<u64>,
    failed: bool,
    via_app: bool,
) -> Result<()> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
    {
        return Err(Error::Configuration(
            "Eval name must use letters, digits, hyphens or underscores",
        ));
    }
    let context = crate::setup::execution(root, true)?;
    let context = Execution {
        pr: pr.or(context.pr),
        ..context
    };
    if via_app {
        let lens = Client::new(
            &Settings::load(root)?.endpoint()?,
            &required("LENS_API_KEY")?,
        )?;
        progress_via_app(
            &lens,
            &ProgressRequest {
                name: name.to_owned(),
                version: context.version,
                pr: context.pr.ok_or(Error::Configuration(
                    "A Lens progress comment requires a pull request event or --pr",
                ))?,
                ci_url: context.ci_url,
                state: if failed {
                    ProgressState::Failed
                } else {
                    ProgressState::Running
                },
            },
        )
        .await?;
        return Ok(());
    }
    from_environment()?.progress(name, &context, failed).await
}

pub async fn progress_via_app(
    lens: &Client,
    progress: &ProgressRequest,
) -> Result<ProgressPublished> {
    let published: ProgressPublished = Client::decode(
        lens.request(
            Method::POST,
            "/lens/github/progress",
            Some(&serde_json::to_value(progress).map_err(Error::Response)?),
            None,
            true,
        )
        .await?,
    )
    .await?;
    if published.comment_url.trim().is_empty() || published.check_url.trim().is_empty() {
        return Err(Error::Infrastructure(
            "Lens did not confirm publishing eval progress",
        ));
    }
    Ok(published)
}

pub async fn publish_file(root: &Path, path: &Path) -> Result<()> {
    publish_file_mode(root, path, false).await
}

pub async fn publish_via_app(lens: &Client, run_ids: &[&str]) -> Result<()> {
    for run_id in run_ids {
        let response: AppReports = Client::decode(
            lens.request(
                Method::POST,
                "/lens/github/report",
                Some(&json!({"run_ids": [run_id]})),
                None,
                true,
            )
            .await?,
        )
        .await?;
        if response.reports.len() != 1 || response.reports[0].run_id != *run_id {
            return Err(Error::Infrastructure(
                "Lens did not confirm publishing the requested eval run",
            ));
        }
    }
    Ok(())
}

pub async fn load_report(lens: &Client, id: &str) -> Result<Report> {
    let run = lens.get(id, false).await?;
    let report = Report {
        run,
        baseline: None,
        trials: Vec::new(),
    };
    let baseline = match &report.summary()?.baseline_run_id {
        Some(id) => Some(lens.get(id, false).await?),
        None => None,
    };
    Ok(Report { baseline, ..report })
}

pub async fn publish_file_mode(root: &Path, path: &Path, via_app: bool) -> Result<()> {
    let key = required("LENS_API_KEY")?;
    let payload: Runs =
        serde_json::from_str(&fs::read_to_string(path)?).map_err(Error::Response)?;
    if payload.runs.is_empty() {
        return Err(Error::Infrastructure("No completed Lens runs to report"));
    }
    let lens = Client::new(&Settings::load(root)?.endpoint()?, &key)?;
    let mut reported_runs = Vec::new();
    if via_app {
        publish_via_app(
            &lens,
            &payload
                .runs
                .iter()
                .map(|run| run.id.as_str())
                .collect::<Vec<_>>(),
        )
        .await?;
        reported_runs = payload.runs;
    } else {
        let github = from_environment()?;
        for run in &payload.runs {
            let report = load_report(&lens, &run.id).await?;
            github.publish(&report, &report.run.version).await?;
            reported_runs.push(report.run);
        }
    }
    if let Some(output) = env_value("GITHUB_OUTPUT") {
        let mut stream = OpenOptions::new().create(true).append(true).open(output)?;
        writeln!(
            stream,
            "passed={}",
            reported_runs.iter().all(|run| run
                .summary
                .as_ref()
                .is_some_and(|summary| summary.gate.passed))
        )?;
        writeln!(
            stream,
            "run-urls={}",
            serde_json::to_string(&reported_runs.iter().map(|run| &run.url).collect::<Vec<_>>())
                .map_err(Error::Response)?
        )?;
    }
    Ok(())
}
