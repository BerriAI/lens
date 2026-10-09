use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    time::Duration,
};

use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    Error, Result,
    client::{Client, segment},
    model::{EvalRun, Report},
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
            let marker = format!("<!-- lens:{} -->", report.run.eval);
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
                    break;
                }
                if comments.len() < 100 {
                    self.request(
                        Method::POST,
                        &format!("/issues/{pr}/comments"),
                        Some(json!({"body":body})),
                    )
                    .await?;
                    break;
                }
            }
        }
        self.request(Method::POST, "/check-runs", Some(json!({"name":format!("Lens / {}", report.run.eval),"head_sha":sha,"status":"completed","conclusion":reporting::conclusion(report)?,"details_url":report.run.url,"output":{"title":format!("Lens / {}", report.run.eval),"summary":body.chars().take(65000).collect::<String>()}}))).await?;
        Ok(())
    }
}

pub async fn publish_file(root: &Path, path: &Path) -> Result<()> {
    let required = |key: &str| {
        env_value(key).ok_or(Error::Invalid {
            message: "GitHub reporting requires",
            value: key.to_owned(),
        })
    };
    let token = required("GITHUB_TOKEN")?;
    let key = required("LENS_API_KEY")?;
    let sha = required("GITHUB_SHA")?;
    let payload: Runs =
        serde_json::from_str(&fs::read_to_string(path)?).map_err(Error::Response)?;
    if payload.runs.is_empty() {
        return Err(Error::Infrastructure("No completed Lens runs to report"));
    }
    let lens = Client::new(&Settings::load(root)?.endpoint()?, &key)?;
    let github = GitHub::new(
        &env_value("GITHUB_API_URL").unwrap_or_else(|| "https://api.github.com".into()),
        &token,
        &required("GITHUB_REPOSITORY")?,
    )?;
    for run in &payload.runs {
        let report = Report {
            run: run.clone(),
            baseline: None,
            trials: Vec::new(),
        };
        let baseline = match &report.summary()?.baseline_run_id {
            Some(id) => Some(lens.get(id, false).await?),
            None => None,
        };
        github.publish(&Report { baseline, ..report }, &sha).await?;
    }
    if let Some(output) = env_value("GITHUB_OUTPUT") {
        let mut stream = OpenOptions::new().create(true).append(true).open(output)?;
        writeln!(
            stream,
            "passed={}",
            payload.runs.iter().all(|run| run
                .summary
                .as_ref()
                .is_some_and(|summary| summary.gate.passed))
        )?;
        writeln!(
            stream,
            "run-urls={}",
            serde_json::to_string(&payload.runs.iter().map(|run| &run.url).collect::<Vec<_>>())
                .map_err(Error::Response)?
        )?;
    }
    Ok(())
}
