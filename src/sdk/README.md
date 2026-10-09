# Lens evals

Run your agent against a versioned Lens dataset, compare each case with main, and use Lens's verdict locally and in CI

For optional help from your coding agent, copy the [eval setup prompt](../../docs/setup-with-agent.md#enable-investigations-signals-or-eval-judging). It inspects this project's agent and existing Lens connection before adding configuration

Run a saved eval from your agent repository:

```python
import os

from lens import Lens

lens = Lens(base_url=os.environ["LENS_BASE_URL"], api_key=os.environ["LENS_API_KEY"])
report = lens.evals.run("moyai-coding-regressions")
report.assert_passed()
```

The saved definition supplies the dataset revision, scorers, gates, and agent input/output mapping. Your local connection profile supplies the agent's address and credentials. The Rust SDK executes requests from your machine or CI runner, then Lens scores the returned output and any correlated traces. Saving a definition does not start tasks

For agents that need custom Python orchestration, the existing callback API remains available:

```python
from lens import Eval, Gate, judge, scorers

evaluation = Eval(
    "agent-regressions",
    task=task,
    data="agent-regressions@7",
    scores=[
        scorers.task_completed(),
        scorers.called_before("run_tests", "open_pr"),
        judge("Did the agent complete the user's request?"),
    ],
    trials=3,
    gate=Gate(regressions=0, critical=0, pass_rate=0.90, cost_per_case=0.50),
)
```

`task` is your async function. A complete example is below

The package is `lens-evals`; the import stays `lens`. Python 3.11+ is supported. HTTP, execution, retries, concurrency, setup, diagnostics, the development server, and GitHub reporting run in Rust under `src/worker/crates/evals-sdk`. `src/worker/crates/evals-python` binds that core to Python with PyO3. Python handles your coroutine and eval-file imports

## Install

This is a private preview. Version `0.1.0a3` is not published to PyPI. Its GitHub wheel release is also pending CI. Generated workflows use `install-from-source: 'true'` to build the SDK from the Action's pinned checkout without a release download. The SDK workflow builds wheels for Linux x86_64, macOS arm64/x86_64, and Windows x86_64. Once published, download the wheel for your platform from the [SDK prerelease](https://github.com/BerriAI/lens/releases/tag/lens-evals-v0.1.0a3), verify it against the accompanying `SHA256SUMS`, then install it in your agent project:

```sh
uv add --dev /absolute/path/to/lens_evals-0.1.0a3-cp311-abi3-PLATFORM.whl
```

Use the actual downloaded filename. Wheels include the Rust implementation, so this install does not need Cargo. A local wheel path must be made available in CI too; do not commit a lockfile pointing only to your laptop's Downloads directory

For contributors, a source install requires Rust 1.99.0 and GitHub access:

```sh
uv add --dev 'lens-evals @ git+https://github.com/BerriAI/lens.git@bad22eb403ea402e00ee04c6e0c16d62693c4261#subdirectory=src/sdk'
```

After registry publication, the intended install is `uv add --dev lens-evals`. The package name avoids a collision with the existing `litellm-lens` server package

## Run a saved eval

This feature requires a Lens server that supports saved `agent_io` definitions and an SDK built from the same reviewed change. Earlier `0.1.0a3` wheels do not contain the named runner. For local development, install `src/sdk` from your checked-out Lens source with Rust 1.99.0 available

Save the eval in Lens first, with an existing dataset ID and pinned revision. Its name is the value passed to `lens.evals.run(...)`; it is not a dataset name

See [saved agent input/output definitions](../../docs/agent-io.md) for complete HTTP and Moyai contracts, the save endpoint, trace requirements, and output-only judging

The definition's `agent_io.connection` names a profile in the current project's `pyproject.toml`:

```toml
[tool.lens]
base_url = "https://lens.example.com"

[tool.lens.connections.coding-agent]
base_url_env = "AGENT_BASE_URL"
auth = "bearer"
token_env = "AGENT_API_KEY"
```

Set those environment variables through your shell or secret manager. The profile contains variable names, not secret values. The saved definition cannot select environment variables or supply an arbitrary destination URL. `auth = "none"` is also supported for an endpoint that does not require authentication

For the built-in Moyai session protocol, use a dedicated eval deployment and a session profile:

```toml
[tool.lens.connections.moyai]
base_url_env = "MOYAI_EVAL_URL"
auth = "moyai_session"
password_env = "MOYAI_EVAL_PASSWORD"
```

The Lens API key reads definitions and datasets and writes eval runs. It is separate from the agent credential and the agent's tracing key. Neither `Lens(...)` nor the named runner writes credentials into environment variables or project files

```sh
uv run lens eval --name moyai-coding-regressions
uv run lens eval --name moyai-coding-regressions --json
```

`--name` runs a saved definition without importing a Python eval file. It cannot be combined with a path or the existing `--eval` filter. `lens doctor` checks file-based evals; it does not preflight a saved definition or prove that a live agent can complete it

Async applications use `report = await lens.evals.arun("moyai-coding-regressions")`. Calling the synchronous method inside an active event loop raises a configuration error. Both methods return the existing `Report`, including `report.url`, per-trial results, baseline links, and `report.assert_passed()`

An existing test suite can call the same API without a task callback:

```python
import os

from lens import Lens


def test_agent_regressions():
    lens = Lens(base_url=os.environ["LENS_BASE_URL"], api_key=os.environ["LENS_API_KEY"])
    lens.evals.run("moyai-coding-regressions").assert_passed()
```

Set `LENS_VERSION` to the build actually deployed at the agent endpoint. Defaults use the checked-out SHA locally or GitHub execution metadata in Actions; those defaults do not prove that the endpoint runs that build. The agent must stamp its own matching version on its traces. Explicit context is available when the deployed build differs from the current checkout:

```python
from lens.config import Execution

report = lens.evals.run(
    "moyai-coding-regressions",
    execution=Execution(version=deployed_sha, branch="feature/my-change"),
)
report.assert_passed()
```

Establish a main run against the deployed main build, then run the same definition against the candidate build. Lens selects a compatible stored baseline and evaluates the saved gates. The SDK does not deploy builds, pick a baseline locally, or treat an accepted agent request as a successful eval

Missing definitions, missing local profiles, unsupported mappings, absent credentials, and incompatible services produce errors. An individual agent request failure is recorded on its trial. A failing quality gate remains a completed `Report`; call `assert_passed()` to enforce it in Python. CLI exits remain `0` for pass, `1` for gate failure, and `2` for configuration or infrastructure failure

## Set up an existing agent

Run this in the agent's repository:

```sh
uv run lens init
```

Setup asks for a dataset, the agent name on your traces, the Lens URL, and optionally an existing async task such as `my_agent.evals:task`. It creates an eval file, adds `[tool.lens]` without replacing other TOML settings, and writes `.github/workflows/lens.yml`. Existing eval files and workflows are protected

The same setup without prompts:

```sh
uv run lens init agent-regressions@7 \
  --project my-agent \
  --base-url https://your-lens-host.example \
  --task my_agent.evals:task
```

These values come from your deployment:

| Value | Source |
|---|---|
| `data="agent-regressions@7"` | Dataset name and revision in Lens's Datasets tab |
| `project="my-agent"` | The `agent.name` attribute your agent exports |
| `LENS_BASE_URL` | Your Lens service URL, without `/ui`; a configured LiteLLM integration URL is also supported |
| `LENS_API_KEY` | A credential allowed to read datasets and create eval runs. Standalone installations can use their Lens admin token; gateway integrations require an authorized team credential. A tracing-only or inference-only key is insufficient |
| `LENS_VERSION` | Defaults to the checked-out Git SHA locally; CI uses `GITHUB_SHA` |
| `LENS_BRANCH` | Defaults to the Git branch locally; CI uses the PR head branch or ref name |
| `AGENT_RUN_URL`, `AGENT_API_KEY` below | Your own agent's API and service credential |

Store `LENS_API_KEY` in your shell or secret manager. Setup does not write credentials to project files

```sh
uv run lens doctor
uv run lens eval
```

`doctor` imports eval files and checks configuration, credentials, selected cases, and the contract-v1 run lookup route. It never calls task functions or creates eval runs. Import-time code in your own modules still runs. A successful doctor check confirms those prerequisites; write access and scoring are exercised by `eval`

## Complete HTTP agent example

This example assumes your agent accepts `POST AGENT_RUN_URL` and immediately replies with `{"session_id": "..."}` while the agent continues running. That URL is your agent's endpoint, not a Lens route. Adapt the request fields and response model to your API

Install `httpx` in your agent project, then save this as `evals/regressions.py`:

```python
import os
from datetime import timedelta
from typing import Final
from uuid import uuid4

import httpx
from pydantic import BaseModel, ConfigDict, Field

from lens import Case, Eval, Gate, Run, judge, scorers


class AcceptedRun(BaseModel):
    model_config = ConfigDict(frozen=True)
    session_id: str = Field(min_length=1)


async def task(case: Case) -> Run:
    async with httpx.AsyncClient(timeout=30) as client:
        response: Final = await client.post(
            os.environ["AGENT_RUN_URL"],
            headers={"Authorization": f"Bearer {os.environ['AGENT_API_KEY']}"},
            json={
                "input": case.input,
                "followups": list(case.followups),
                "session_id": uuid4().hex,
                "eval_mode": True,
            },
        )
        response.raise_for_status()
        accepted: Final = AcceptedRun.model_validate_json(response.content)
    return Run(trace={"session.id": accepted.session_id})


evaluation: Final = Eval(
    "agent-regressions",
    task=task,
    data="agent-regressions@7",
    scores=[
        scorers.task_completed(),
        scorers.called_before("run_tests", "open_pr"),
        judge("Did the agent complete the user's request?"),
    ],
    baseline="main",
    trials=3,
    concurrency=8,
    timeout_per_trial=timedelta(minutes=10),
    gate=Gate(regressions=0, critical=0, pass_rate=0.90, cost_per_case=0.50),
)
```

The matching configuration is:

```toml
[tool.lens]
project = "my-agent"
evals = "evals/"
base_url = "https://your-lens-host.example"
```

Your agent must implement `eval_mode`, run in its sandbox, and export `session.id`, `agent.name`, and `deployment.environment="lens-eval"` on its trace. It must stamp `agent.version` from its own build SHA, never from a version supplied by the eval request. Lens records a trial error if the agent, environment, or build version does not match the eval run. Passing `eval_mode` does not disable side effects by itself

Return as soon as the agent accepts the run and supplies its trace reference. The task does not poll for completion. Lens waits until the trace's root span ends or it has been idle for 120 seconds, capped by `timeout_per_trial`, including time spent accepting the agent run. The SDK submits the reference and waits for Lens's result

You can return `Run(trace={"trace_id": accepted_trace_id})` instead. Supply exactly one trace reference. `cost_usd=None` asks Lens to derive cost from the trace. Tasks return trace references, never scores

## Cases, scorers, and gates

`case.input` is the first user message. `case.followups` contains subsequent user messages. `case.meta` contains dataset metadata, and `case.expected` contains the expected outcome if supplied

| Scorer | What Lens evaluates |
|---|---|
| `scorers.task_completed()` | Whether the task was completed |
| `scorers.called_before("run_tests", "open_pr")` | Whether the first tool precedes calls to the second |
| `judge("Your rubric", model="your-judge-model")` | A custom rubric scored by Lens; omit `model` for its default |

Every configured gate condition must pass. `None` disables a condition. Scorer minimums use the scorer's name, with repeated judges named `judge_1`, `judge_2` in declaration order:

```python
strict = evaluation.gate(Gate(regressions=0, min={"task_completed": 0.95}))
small = evaluation.subset(case_ids=["case-a", "case-b"])
finding = evaluation.subset(finding=123)
```

These return new evals. Subsets get a separate eval name, so a partial run does not replace the full eval's baseline

Finding IDs may be integers or strings. The SDK matches their string form against `DatasetCase.meta["finding_id"]`, with `source.finding_id` accepted for older dataset responses. The dataset endpoint supplies the mapping; there is no separate finding lookup route

## Read a result

```python
from evals.regressions import evaluation

report = evaluation.run()
print(report.passed, report.total, report.cost_per_case)
print(report.scores, report.errors, report.url)
for case in report.regressions:
    print(case.case_id, case.title, case.baseline_url, case.candidate_url)
report.assert_passed()
```

Inside an event loop, use `report = await evaluation.arun()`. `assert_passed()` raises `lens.GateFailed` if Lens's gate fails

The SDK trusts the server's `Summary`. It does not compute production scores, choose a baseline, or recompute the gate. Individual task failures become case results. Network, authorization, or scoring-service failures are infrastructure errors

## CLI

```sh
uv run lens eval
uv run lens eval evals/regressions.py
uv run lens eval --eval agent-regressions --json
uv run lens eval --name moyai-coding-regressions --json
uv run lens doctor --json
uv run lens eval --ci --json
```

`lens eval` exits `0` for passing gates, `1` for a failed gate, and `2` for configuration or infrastructure problems. `--json` reserves stdout for the report; task output goes to stderr. `doctor` exits `0` when checks pass and `2` when attention is needed

Each normal invocation creates a separate execution, even at the same commit. Safe HTTP retries reuse the run's idempotency key. Set `LENS_EXECUTION_ID` only when intentionally resuming the same execution

## GitHub Action

Start from the agent’s **Connect GitHub** button in Lens to authorize the [GitHub App](../../docs/github-app.md) and save a repository connection. Then choose **Set up PR evals**. The [eval setup](../../docs/github-evals.md) selects an eval, prepares the workflow and adapter, and checks for the first PR eval received from that repository

Setup generates a workflow for main pushes, same-repository PRs, and manual runs. Add `LENS_API_KEY` as a repository secret and `LENS_BASE_URL` as a repository variable. Enable private Action access for the consuming repository. Add your agent's dependencies, sandbox startup, and service credentials to that workflow. Run it on main first to establish the baseline, then open a same-repository PR to receive the comparison comment

For a project using uv, the relevant steps are:

```yaml
permissions:
  contents: read
  checks: write
  pull-requests: write

steps:
  - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262
  - uses: actions/setup-python@a26af69be951a213d495a4c3e4e4022e16d87065
    with:
      python-version: '3.11'
  - run: python -m pip install uv==0.10.9
  - run: uv sync --frozen
  - uses: BerriAI/lens/src/sdk/action@bad22eb403ea402e00ee04c6e0c16d62693c4261
    with:
      python: .venv/bin/python
      install-from-source: 'true'
      api-key: ${{ secrets.LENS_API_KEY }}
      base-url: ${{ vars.LENS_BASE_URL }}
```

The `python` input selects the agent's environment for execution and reporting. With `install-from-source: 'true'`, the Action installs Rust 1.99.0 through the runner's existing rustup and builds its own checked-out SDK using the pinned maturin build backend and Cargo lockfile. No release token is needed. GitHub-hosted runners include rustup; self-hosted runners need it installed

For a saved eval, set `eval-name` instead of `path`. Commit its local connection profile and provide the named environment variables as CI secrets or variables. Set `LENS_VERSION` from the agent build deployed by your workflow. The Action does not deploy or update the agent

Use the pinned Action with the same saved eval name:

```yaml
- uses: BerriAI/lens/src/sdk/action@bad22eb403ea402e00ee04c6e0c16d62693c4261
  env:
    AGENT_BASE_URL: ${{ vars.AGENT_BASE_URL }}
    AGENT_API_KEY: ${{ secrets.AGENT_API_KEY }}
    LENS_VERSION: ${{ steps.deploy.outputs.agent-build-sha }}
  with:
    eval-name: my-agent-regressions
    api-key: ${{ secrets.LENS_API_KEY }}
    base-url: ${{ vars.LENS_BASE_URL }}
    python: .venv/bin/python
    install-from-source: 'true'
```

The example assumes your deployment step publishes `agent-build-sha`. The pinned source commit contains the named runner; use it after reviewing this feature for your deployment. Supplying both `eval-name` and `path` fails before installation; an older installed wheel without named-run support produces an actionable error

The default `install-from-source: 'false'` reuses an installed `0.1.0a3` native package or downloads the matching release wheel and verifies its SHA-256 checksum. Once wheels are published, use that mode with `sdk-token: ${{ secrets.LENS_SDK_TOKEN }}` when the consuming repository needs separate contents-read access to the internal SDK release

For an agent connected to the Lens GitHub App, set `report-via-app: 'true'`. The publish step sends run IDs to Lens, which checks the saved repository connection and GitHub workflow before publishing the server’s verdict as the App. This mode only needs `contents: read` for the workflow token. Each eval run gets its own report, and publication retries reuse it. The equivalent standalone command is `python -m lens.github runs.json --via-app`

Without that input, the Action updates its own GitHub Actions bot comment using the write permissions shown above. Both modes create `Lens / <eval-name>` and expose `passed` and `run-urls`. The comment includes baseline/candidate pass counts, costs, scores, broken cases, and Lens-provided trace links

On a PR, the check is **neutral** only when there is no comparable main baseline and the server gate passes every configured absolute condition (`pass_rate`, `cost_per_case`, and scorer `min`). A failed absolute condition produces a **failure** check and a red job. With a baseline, the server gate determines success or failure. The SDK does not recompute the gate

## Try it without an agent

```sh
uv run lens init --demo
uv run lens dev-server
```

In a second terminal:

```sh
LENS_API_KEY=lens-dev LENS_VERSION=demo LENS_BRANCH=main uv run lens doctor
LENS_API_KEY=lens-dev LENS_VERSION=demo LENS_BRANCH=main uv run lens eval
```

The demo runs 36 synthetic cases with three trials each. References starting with `pass` or `fail` are reserved synthetic traces that are already closed; those containing `pass` score successfully. Change `pass-` to `fail-` in the generated task to see the gate fail, then restore it to see green again. No model or real agent is called. Demo setup deliberately creates no CI workflow

For an asynchronous local agent, return an ordinary accepted reference, such as `accepted-pass-123`. The dev server keeps the run in `scoring` until it receives trace activity at its test-only route:

```sh
curl -X POST http://127.0.0.1:8765/_dev/traces \
  -H 'Authorization: Bearer lens-dev' \
  -H 'X-Lens-Contract: 1' \
  -H 'Content-Type: application/json' \
  -d '{"trace":{"attribute":"session.id","value":"accepted-pass-123"},"agent_version":"demo","root_ended":true}'
```

Send this from the fake agent or a second terminal; the eval task only submits the run. With `root_ended: false`, each update counts as activity and the stub waits for 120 seconds of inactivity. A mismatching build version or an unclosed trace at the deadline becomes a trial error. The cap comes from the run's `timeout_per_trial_ms`, which the SDK sends from `Eval(timeout_per_trial=...)`; it defaults to 1,200 seconds when omitted. Registered trace state takes precedence over synthetic prefix shortcuts

The development server is loopback-only and keeps state in memory. `--dataset-file /path/to/cases.json` accepts a downloaded `EvalCases` response for integration testing. Its verdicts remain synthetic

## Implementation and integration status

The native core is in [`src/worker/crates/evals-sdk`](../worker/crates/evals-sdk), with PyO3 bindings in [`src/worker/crates/evals-python`](../worker/crates/evals-python). The Python package contains immutable public values, generated Pydantic wire models, discovery, and coroutine handling

The Lens server owns lifecycle routes, trace scoring, baseline selection, and the comparison UI. This package provides the SDK, CLI, setup, Action, and development server

The callback SDK uses `X-Lens-Contract: 1`; named evals use `X-Lens-Contract: 2` for saved agent I/O and direct outputs. Rust types in `lens-contract` define `schema/lens.v1.json`, which generates the Python wire models. Shared golden fixtures live at `src/worker/crates/contract/fixtures/lens_eval/`. From the repository root, run `npm run generate:eval-contract` after changing the canonical contract

Version `0.1.0a3` sends a positive integer `timeout_per_trial_ms` on every create-run request. The server default for clients that omit it is 1,200,000 ms. Lens enforces this cap while waiting for the trace and exposes finding IDs in `DatasetCase.meta["finding_id"]`

See [verification](validation/verification.md) for exact evidence and remaining integration work. The full real-gateway transport example is [`examples/live_gateway.py`](examples/live_gateway.py); it makes a model call and exports a trace using its own `AGENT_BUILD_SHA`, but does not exercise a deployed custom agent
