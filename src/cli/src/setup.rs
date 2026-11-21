use std::{
    fs,
    io::{self, IsTerminal, Write},
    path::Path,
};

use lens_evals_sdk::{
    client,
    setup::{Settings, env_value},
};
use toml_edit::{DocumentMut, Item, Table};

use crate::{Error, Result};

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
