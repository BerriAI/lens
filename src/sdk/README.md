# Lens evals

Run your agent against a versioned Lens dataset, compare each case with main, and use Lens's verdict locally and in CI

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

This is a private preview. Version `0.1.0a2` is not published to PyPI. The SDK workflow builds wheels for Linux x86_64, macOS arm64/x86_64, and Windows x86_64. Download the wheel for your platform from a successful [Lens SDK workflow run](https://github.com/BerriAI/lens/actions/workflows/lens-sdk.yml), then install it in your agent project:

```sh
uv add --dev /absolute/path/to/lens_evals-0.1.0a2-cp311-abi3-PLATFORM.whl
```

Use the actual downloaded filename. Wheels include the Rust implementation, so this install does not need Cargo. A local wheel path must be made available in CI too; do not commit a lockfile pointing only to your laptop's Downloads directory

For contributors, a source install requires Rust 1.99.0 and GitHub access:

```sh
uv add --dev 'lens-evals @ git+ssh://git@github.com/BerriAI/lens.git@f96e812b57f27063f895f635e495de4931ee75ee#subdirectory=packages/sdk'
```

After registry publication, the intended install is `uv add --dev lens-evals`. The package name avoids a collision with the existing `litellm-lens` server package

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
| `LENS_BASE_URL` | Your Lens gateway URL, without `/ui` |
| `LENS_API_KEY` | A key allowed to access Lens dataset and eval routes; an inference-only key is insufficient |
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

This example assumes your agent accepts `POST AGENT_RUN_URL`, waits for its work and trace export to finish, and replies with `{"session_id": "...", "cost_usd": 0.12}`. That URL is your agent's endpoint, not a Lens route. Adapt the request fields and response model to your API

Install `httpx` in your agent project, then save this as `evals/regressions.py`:

```python
import os
from datetime import timedelta
from typing import Final
from uuid import uuid4

import httpx
from pydantic import BaseModel, ConfigDict, Field

from lens import Case, Eval, Gate, Run, judge, scorers
from lens.config import Execution


class CompletedRun(BaseModel):
    model_config = ConfigDict(frozen=True)
    session_id: str = Field(min_length=1)
    cost_usd: float | None = Field(default=None, ge=0)


async def task(case: Case) -> Run:
    context: Final = Execution.github() if os.environ.get("GITHUB_ACTIONS") == "true" else Execution.local()
    async with httpx.AsyncClient(timeout=600) as client:
        response: Final = await client.post(
            os.environ["AGENT_RUN_URL"],
            headers={"Authorization": f"Bearer {os.environ['AGENT_API_KEY']}"},
            json={
                "input": case.input,
                "followups": list(case.followups),
                "session_id": uuid4().hex,
                "eval_mode": True,
                "trace_attributes": {
                    "agent.name": "my-agent",
                    "agent.version": context.version,
                    "deployment.environment": "lens-eval",
                },
            },
        )
        response.raise_for_status()
        completed: Final = CompletedRun.model_validate_json(response.content)
    return Run(trace={"session.id": completed.session_id}, cost_usd=completed.cost_usd)


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

Your agent must implement `eval_mode`, run in its sandbox, and export `session.id`, `agent.name`, and the candidate `agent.version` on its trace. Passing a flag does not disable side effects by itself. The API must execute the version being evaluated; a remote service running yesterday's build cannot test the PR's changes

Return only after the agent completes and its trace is exported. If your API returns immediately with a job ID, waiting for that job belongs in your task function. Contract v1 has no universal agent-completion endpoint. The SDK handles Lens run polling, but cannot remove your agent's completion protocol

You can return `Run(trace={"trace_id": completed_trace_id})` instead. Supply exactly one trace reference. `cost_usd=None` asks Lens to derive cost from the trace. Tasks return trace references, never scores

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
finding = evaluation.subset(finding="finding-id-from-lens")
```

These return new evals. Subsets get a separate eval name, so a partial run does not replace the full eval's baseline

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
uv run lens doctor --json
uv run lens eval --ci --json
```

`lens eval` exits `0` for passing gates, `1` for a failed gate, and `2` for configuration or infrastructure problems. `--json` reserves stdout for the report; task output goes to stderr. `doctor` exits `0` when checks pass and `2` when attention is needed

Each normal invocation creates a separate execution, even at the same commit. Safe HTTP retries reuse the run's idempotency key. Set `LENS_EXECUTION_ID` only when intentionally resuming the same execution

## GitHub Action

Setup generates a workflow for main pushes and same-repository PRs. Add `LENS_API_KEY` as a repository secret and `LENS_BASE_URL` as a repository variable. Add your agent's dependencies, sandbox startup, and service credentials to that workflow

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
  - uses: BerriAI/lens/packages/sdk/action@18bf2b38bc4239d1976727ed6bc8e211dd4e52f9
    with:
      python: .venv/bin/python
      api-key: ${{ secrets.LENS_API_KEY }}
      base-url: ${{ vars.LENS_BASE_URL }}
```

The `python` input selects the agent's environment for execution and reporting. The Action uses an installed `0.1.0a2` native package or builds its bundled SDK into that environment, using Rust 1.99.0 on a runner with rustup. Hosted Ubuntu runners support that fallback. Private Action access must be enabled for consuming repositories

The Action updates its own PR comment, creates `Lens / <eval-name>`, and exposes `passed` and `run-urls`. The comment includes baseline/candidate pass counts, costs, scores, broken cases, and Lens-provided trace links

With no comparable main baseline, the PR check is **neutral**, as specified in contract D. An absolute gate can still fail and make the CLI and Action job red. Run the eval on main to establish a comparable baseline

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

The demo runs 36 synthetic cases with three trials each. Its development server treats trace references containing `pass` as successful. Change `pass-` to `fail-` in the generated task to see the gate fail, then restore it to see green again. No model or real agent is called. Demo setup deliberately creates no CI workflow

The development server is loopback-only and keeps state in memory. `--dataset-file /path/to/cases.json` accepts a downloaded `EvalCases` response for integration testing. Its verdicts remain synthetic

## Implementation and integration status

The native core is in [`src/worker/crates/evals-sdk`](../worker/crates/evals-sdk), with PyO3 bindings in [`src/worker/crates/evals-python`](../worker/crates/evals-python). The Python package contains immutable public values, generated Pydantic wire models, discovery, and coroutine handling

Ishaan owns the production lifecycle routes, trace scoring, baseline selection, canonical `lens-contract` schema, and Lens comparison UI. Moe owns this SDK, CLI, setup, Action, and development server. No production server routes are added by this package

Contract B routes, bodies, status codes, error codes, `X-Lens-Contract: 1`, and Summary semantics stay unchanged. Until Ishaan lands `schema/lens.v1.json`, generation uses the appendix schema. CI validates the shared golden fixtures at `src/worker/crates/contract/fixtures/lens_eval/` when present, and the provisional fixture copy otherwise. Once available, the canonical Rust types should replace the SDK's provisional wire structs

See [verification](validation/verification.md) for exact evidence and remaining integration work. The full real-gateway transport example is [`examples/live_gateway.py`](examples/live_gateway.py); it makes a model call and exports a trace, but does not exercise a deployed custom agent
