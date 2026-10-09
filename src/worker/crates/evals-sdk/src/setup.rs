use std::{
    env, fs,
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    process::Command,
};

use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use toml_edit::{DocumentMut, Item, Table};
use uuid::Uuid;

use crate::{Error, Result, client, model::Execution};

pub const ACTION: &str = "BerriAI/lens/src/sdk/action@ba921dfbf3cd10a71b43bf1d756ef56ffc0fe69b";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub project: String,
    #[serde(default = "eval_path")]
    pub evals: String,
    pub base_url: String,
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

#[derive(Parser)]
#[command(
    name = "lens",
    about = "Evaluate your agent against Lens datasets",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Operation,
}

#[derive(Subcommand, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
pub enum Operation {
    Eval {
        path: Option<PathBuf>,
        #[arg(long = "eval")]
        name: Option<String>,
        #[arg(long)]
        ci: bool,
        #[arg(long)]
        json: bool,
    },
    Doctor {
        path: Option<PathBuf>,
        #[arg(long = "eval")]
        name: Option<String>,
        #[arg(long)]
        ci: bool,
        #[arg(long)]
        json: bool,
    },
    Init {
        dataset: Option<String>,
        #[arg(long)]
        project: Option<String>,
        #[arg(long)]
        base_url: Option<String>,
        #[arg(long)]
        task: Option<String>,
        #[arg(long)]
        demo: bool,
        #[arg(long, default_value = ACTION)]
        action_ref: String,
    },
    DevServer {
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        #[arg(long, default_value_t = 8765)]
        port: u16,
        #[arg(long)]
        dataset_file: Option<PathBuf>,
    },
    #[command(hide = true)]
    Report { file: PathBuf },
}

pub fn parse(arguments: &[String]) -> serde_json::Value {
    match Cli::try_parse_from(arguments) {
        Ok(cli) => serde_json::to_value(cli.command).expect("CLI serializes"),
        Err(error) => {
            serde_json::json!({"command":"help", "text": error.to_string(), "code": error.exit_code()})
        }
    }
}

pub struct InitOptions {
    pub dataset: Option<String>,
    pub project: Option<String>,
    pub base_url: Option<String>,
    pub task: Option<String>,
    pub demo: bool,
    pub action: String,
}

fn answer(
    value: Option<String>,
    label: &str,
    interactive: bool,
    prompt: &mut impl FnMut(&str) -> Result<String>,
) -> Result<String> {
    match value.filter(|value| !value.is_empty()) {
        Some(value) => Ok(value),
        None if interactive => prompt(label),
        None => Ok(String::new()),
    }
}

pub fn prompt(label: &str) -> Result<String> {
    print!("{label}: ");
    io::stdout().flush()?;
    let mut line = String::new();
    if io::stdin().read_line(&mut line)? == 0 {
        return Err(Error::Configuration("Setup cancelled before writing files"));
    }
    Ok(line.trim().to_owned())
}

pub fn initialize(root: &Path, options: InitOptions) -> Result<String> {
    initialize_with(root, options, io::stdin().is_terminal(), &mut prompt)
}

pub fn initialize_with(
    root: &Path,
    options: InitOptions,
    interactive: bool,
    prompt: &mut impl FnMut(&str) -> Result<String>,
) -> Result<String> {
    if options.demo
        && (options.task.is_some()
            || options
                .dataset
                .as_ref()
                .is_some_and(|value| value != "demo@1"))
    {
        return Err(Error::Configuration(
            "--demo uses the synthetic demo@1 dataset and its generated task",
        ));
    }
    let previous = Settings::load(root)?;
    let dataset = answer(
        options
            .dataset
            .or_else(|| options.demo.then(|| "demo@1".to_owned())),
        "Dataset (name@revision)",
        interactive,
        prompt,
    )?;
    let project = answer(
        options
            .project
            .or_else(|| (!previous.project.is_empty()).then(|| previous.project.clone()))
            .or_else(|| options.demo.then(|| "demo".to_owned())),
        "Agent name (agent.name on your traces)",
        interactive,
        prompt,
    )?;
    let base = answer(
        options
            .base_url
            .or_else(|| (!previous.base_url.is_empty()).then(|| previous.base_url.clone()))
            .or_else(|| options.demo.then(|| "http://127.0.0.1:8765".to_owned()))
            .or_else(|| env_value("LENS_BASE_URL")),
        "Lens base URL",
        interactive,
        prompt,
    )?;
    let task = answer(
        options.task,
        "Existing async task (package.module:function, or Enter for a scaffold)",
        interactive && !options.demo,
        prompt,
    )?;
    let (name, revision) = dataset
        .split_once('@')
        .map_or((dataset.as_str(), None), |(name, rev)| (name, Some(rev)));
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_-".contains(&byte))
        || !name.as_bytes()[0].is_ascii_alphanumeric()
        || revision.is_some_and(|text| text.parse::<u64>().ok().is_none_or(|value| value == 0))
    {
        return Err(Error::Configuration(
            "Pass a lowercase dataset name or name@positive-revision, or run lens init in a terminal",
        ));
    }
    if project.is_empty() {
        return Err(Error::Configuration(
            "Pass --project with your agent.name, or run lens init in a terminal",
        ));
    }
    if !options.action.contains('@')
        || !options.action.contains('/')
        || !options
            .action
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_./-@".contains(&byte))
    {
        return Err(Error::Configuration(
            "Action reference must have the form owner/repo[/path]@ref",
        ));
    }
    let base = if base.is_empty() {
        base
    } else {
        client::endpoint(&base)?
    };
    let config = root.join("pyproject.toml");
    let target = root
        .join(&previous.evals)
        .join(format!("{}.py", name.replace('-', "_")));
    let workflow = root.join(".github/workflows/lens.yml");
    for file in [&target, &workflow]
        .into_iter()
        .take(if options.demo { 1 } else { 2 })
    {
        if file.exists() {
            return Err(Error::Invalid {
                message: "Refusing to overwrite",
                value: file.display().to_string(),
            });
        }
    }
    let content = eval_file(name, &dataset, &task, options.demo)?;
    let existing = if config.exists() {
        fs::read_to_string(&config)?
    } else {
        String::new()
    };
    let mut document = existing
        .parse::<DocumentMut>()
        .map_err(|_| Error::Configuration("Invalid pyproject.toml"))?;
    if document.get("tool").is_none() {
        document["tool"] = Item::Table(Table::new());
    }
    if !document["tool"].is_table_like() {
        return Err(Error::Configuration("[tool] must be a TOML table"));
    }
    if document["tool"].get("lens").is_none() {
        document["tool"]["lens"] = Item::Table(Table::new());
    }
    if !document["tool"]["lens"].is_table_like() {
        return Err(Error::Configuration("[tool.lens] must be a TOML table"));
    }
    for (key, value) in [
        ("project", project.as_str()),
        ("evals", previous.evals.as_str()),
        ("base_url", base.as_str()),
    ] {
        if !value.is_empty() {
            document["tool"]["lens"][key] = toml_edit::value(value);
        }
    }
    fs::create_dir_all(target.parent().expect("eval parent"))?;
    if !options.demo {
        fs::create_dir_all(workflow.parent().expect("workflow parent"))?;
    }
    fs::write(&target, content)?;
    fs::write(&config, document.to_string())?;
    if options.demo {
        return Ok(format!(
            "Created {}. Demo scores are synthetic; no agent or model is called\nStart the local server: uv run lens dev-server\nIn another terminal:\n  LENS_API_KEY=lens-dev LENS_VERSION=demo LENS_BRANCH=main uv run lens doctor\n  LENS_API_KEY=lens-dev LENS_VERSION=demo LENS_BRANCH=main uv run lens eval",
            target.display()
        ));
    }
    fs::write(&workflow, workflow_file(&options.action, root))?;
    Ok(format!(
        "Created {} and {}\n{}Set LENS_API_KEY in your shell, then run: uv run lens doctor\nWhen ready: uv run lens eval\nFor CI, set the LENS_API_KEY secret and LENS_BASE_URL variable\nAllow this repository to use the Lens Action, and add your agent's dependencies and startup steps\nRun the Lens workflow on main to establish a baseline, then open a same-repository pull request",
        target.display(),
        workflow.display(),
        if task.is_empty() {
            "Implement task(): submit the agent run, then return its accepted trace reference\n"
        } else {
            ""
        }
    ))
}

fn eval_file(name: &str, dataset: &str, task: &str, demo: bool) -> Result<String> {
    let imports = if task.is_empty() {
        "from lens import Case, Eval, Gate, Run, scorers"
    } else {
        "from lens import Eval, Gate, scorers"
    };
    let adapter = if !task.is_empty() {
        let (module, function) = task
            .split_once(':')
            .ok_or(Error::Configuration("Use --task package.module:function"))?;
        if module.split('.').chain([function]).any(|name| {
            name.is_empty()
                || name.bytes().enumerate().any(|(index, byte)| {
                    !(byte.is_ascii_alphabetic()
                        || byte == b'_'
                        || (index > 0 && byte.is_ascii_digit()))
                })
                || [
                    "False", "None", "True", "and", "as", "assert", "async", "await", "break",
                    "class", "continue", "def", "del", "elif", "else", "except", "finally", "for",
                    "from", "global", "if", "import", "in", "is", "lambda", "nonlocal", "not",
                    "or", "pass", "raise", "return", "try", "while", "with", "yield",
                ]
                .contains(&name)
        }) {
            return Err(Error::Configuration(
                "Use --task package.module:function with valid Python identifiers",
            ));
        }
        format!("from {module} import {function} as task")
    } else if demo {
        "async def task(case: Case) -> Run:\n    return Run(trace={\"session.id\": f\"pass-{uuid4().hex}\"}, cost_usd=0.01)".to_owned()
    } else {
        "async def task(case: Case) -> Run:\n    raise NotImplementedError(\"Submit your agent run and return the accepted Run(trace={'session.id': session_id})\")".to_owned()
    };
    let extra = if demo { "from uuid import uuid4\n" } else { "" };
    Ok(format!(
        "from typing import Final\n{extra}\n{imports}\n\n\n{adapter}\n\n\nevaluation: Final = Eval(\n    {name:?},\n    task=task,\n    data={dataset:?},\n    scores=[scorers.task_completed()],\n    trials=3,\n    gate=Gate(pass_rate=1),\n)\n"
    ))
}

pub fn workflow_file(action: &str, root: &Path) -> String {
    let uv = root.join("uv.lock").exists();
    let install = if uv {
        "      - run: python -m pip install uv==0.10.9\n      - run: uv sync --frozen\n"
    } else if root.join("requirements.txt").exists() {
        "      - run: python -m pip install -r requirements.txt\n"
    } else if fs::read_to_string(root.join("pyproject.toml"))
        .ok()
        .and_then(|text| toml::from_str::<toml::Value>(&text).ok())
        .is_some_and(|document| document.get("project").is_some())
    {
        "      - run: python -m pip install -e .\n"
    } else {
        ""
    };
    let python = if uv {
        "          python: .venv/bin/python\n"
    } else {
        ""
    };
    format!(
        "name: Lens\non:\n  push:\n    branches: [main]\n  pull_request:\n  workflow_dispatch:\npermissions:\n  contents: read\n  checks: write\n  pull-requests: write\nconcurrency:\n  group: lens-${{{{ github.event.pull_request.number || github.ref }}}}\n  cancel-in-progress: true\njobs:\n  eval:\n    if: github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262\n      - uses: actions/setup-python@a26af69be951a213d495a4c3e4e4022e16d87065\n        with:\n          python-version: '3.11'\n{install}      - uses: {action}\n        with:\n{python}          install-from-source: 'true'\n          api-key: ${{{{ secrets.LENS_API_KEY }}}}\n          base-url: ${{{{ vars.LENS_BASE_URL }}}}\n"
    )
}
