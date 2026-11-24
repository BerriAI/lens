use std::{collections::BTreeMap, env, fs, path::Path, process::Command};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Error, Result, client, model::Execution};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub project: String,
    #[serde(default = "eval_path")]
    pub evals: String,
    pub base_url: String,
    #[serde(skip_serializing)]
    pub connections: BTreeMap<String, crate::agent::Connection>,
}
fn eval_path() -> String {
    "evals/".to_owned()
}

impl Settings {
    pub fn load(root: &Path) -> Result<Self> {
        let path = root.join("pyproject.toml");
        if !path.exists() {
            return Ok(Self {
                evals: eval_path(),
                ..Self::default()
            });
        }
        let document: toml::Value = toml::from_str(&fs::read_to_string(path)?)?;
        match document.get("tool").and_then(|tool| tool.get("lens")) {
            Some(value) => value.clone().try_into().map_err(Error::Toml),
            None => Ok(Self {
                evals: eval_path(),
                ..Self::default()
            }),
        }
    }
    pub fn endpoint(&self) -> Result<String> {
        client::endpoint(&env_value("LENS_BASE_URL").unwrap_or_else(|| self.base_url.clone()))
    }
}

pub fn env_value(key: &str) -> Option<String> {
    env::var(key).ok().filter(|value| !value.trim().is_empty())
}

fn git(root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git").args(args).current_dir(root).output()?;
    if !output.status.success() {
        return Err(Error::Configuration(
            "Run inside a Git checkout with a commit, or set LENS_VERSION and LENS_BRANCH",
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub fn execution(root: &Path, ci: bool) -> Result<Execution> {
    if !ci {
        return Ok(Execution {
            baseline_run_id: env_value("LENS_BASELINE_RUN_ID"),
            version: match env_value("LENS_VERSION") {
                Some(value) => value,
                None => git(root, &["rev-parse", "HEAD"])?,
            },
            branch: match env_value("LENS_BRANCH") {
                Some(value) => value,
                None => git(root, &["rev-parse", "--abbrev-ref", "HEAD"])?,
            },
            pr: None,
            ci_url: String::new(),
            identity: env_value("LENS_EXECUTION_ID")
                .unwrap_or_else(|| Uuid::new_v4().simple().to_string()),
        });
    }
    let required = |name: &'static str| {
        env_value(name).ok_or(Error::Invalid {
            message: "Missing GitHub Actions environment variable",
            value: name.to_owned(),
        })
    };
    let event: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(required("GITHUB_EVENT_PATH")?)?)
            .map_err(Error::Response)?;
    let run = required("GITHUB_RUN_ID")?;
    Ok(Execution {
        baseline_run_id: env_value("LENS_BASELINE_RUN_ID"),
        version: required("GITHUB_SHA")?,
        branch: match env_value("GITHUB_HEAD_REF") {
            Some(value) => value,
            None => required("GITHUB_REF_NAME")?,
        },
        pr: event
            .get("pull_request")
            .and_then(|pull| pull.get("number"))
            .and_then(serde_json::Value::as_u64),
        ci_url: format!(
            "{}/{}/actions/runs/{run}",
            env_value("GITHUB_SERVER_URL")
                .unwrap_or_else(|| "https://github.com".to_owned())
                .trim_end_matches('/'),
            required("GITHUB_REPOSITORY")?
        ),
        identity: format!(
            "{run}:{}",
            env_value("GITHUB_RUN_ATTEMPT").unwrap_or_else(|| "1".to_owned())
        ),
    })
}
