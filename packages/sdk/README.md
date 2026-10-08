# Lens evals Python SDK

Run an agent against a versioned Lens dataset, repeat each case, and use Lens scores to gate a release

```python
from lens import Case, Eval, Gate, Run, scorers
```

The package is `lens-evals`; the import is `lens`. It includes the Python SDK, `lens` CLI, a local development server, and a GitHub Action

**Preview:** the SDK works against the local contract server. Real dataset downloads, gateway model calls, and trace uploads have also been verified. Production eval scoring is waiting on the Rust eval API. See [verification](validation/verification.md) for the evidence and limits

## Install

Python 3.11+ is required. The package is not on PyPI yet; install from this repository using a GitHub account with access

```bash
uv init --bare --python 3.11 lens-evals-demo
cd lens-evals-demo
uv add --dev "lens-evals[dev-server] @ git+https://github.com/BerriAI/lens.git@main#subdirectory=packages/sdk"
```

For an existing project, run just `uv add`. The `dev-server` extra is needed only for the local server. From a Lens checkout, the equivalent source install is `uv add --dev './packages/sdk[dev-server]'`

Once the package is published, installation will be `uv add --dev lens-evals`

## Run your first eval

This example needs no gateway credentials or model calls. The local server supplies 36 demo cases and synthetic scores

Add this to `pyproject.toml`

```toml
[tool.lens]
project = "demo"
evals = "evals/"
base_url = "http://127.0.0.1:8765"
```

Start the server in one terminal, from the project directory

```bash
uv run lens dev-server
```

In another terminal in the same directory, create `evals/demo.py`

```python
from typing import Final
from uuid import uuid4

from lens import Case, Eval, Gate, Run, scorers


async def task(case: Case) -> Run:
    return Run(trace={"session.id": f"pass-{uuid4().hex}"}, cost_usd=0.01)


evaluation: Final = Eval(
    "demo",
    task=task,
    data="demo@1",
    scores=[scorers.task_completed()],
    trials=3,
    gate=Gate(pass_rate=0.9),
)
```

Run it

```bash
export LENS_API_KEY=lens-dev
export LENS_VERSION=demo-v1
export LENS_BRANCH=main
uv run lens eval
```

The CLI uploads 108 trial results and reports `demo: 36/36 passed`, zero trial errors, and `gate passed`. It exits 0

The demo task returns a synthetic reference. The dev server marks references containing `pass` as successful; it does not inspect a trace or evaluate an answer. Replace `pass-` with `fail-` in the task and rerun with `LENS_BRANCH=feature LENS_VERSION=demo-v2 uv run lens eval` to see a failed gate and exit 1 against the previous main baseline

The server keeps data in memory until it stops. Its run URLs return authenticated JSON, not the Lens Runs UI

## Where inputs and IDs come from

Your task receives one `Case` for each included dataset case, once per trial

| Value | Source |
| --- | --- |
| `data="demo@1"` | Dataset name and revision in Lens; the dev server creates `demo` revision 1 |
| `case.id` | The dataset case ID |
| `case.input` | The first user message in that case |
| `case.followups` | Later user messages, in order |
| `case.meta` | Dataset metadata, as string key/value pairs |
| `case.expected` | The case's expected answer, or an empty string |
| `Run.trace` | A session or trace ID produced by your agent's tracing |
| `Run.cost_usd` | Optional total cost for that trial, supplied by your adapter |
| `[tool.lens].project` | Your agent's `agent.name` trace attribute |
| Version and branch | Local Git checkout, or `LENS_VERSION` / `LENS_BRANCH`; GitHub context when using `--ci` |

Use `name@revision` to keep the dataset fixed. With just `name`, the SDK resolves the current revision once at the start of the run. It also supports older gateways that expose the dataset list but lack the resolve endpoint

## Connect your agent

The adapter is an `async def task(case: Case) -> Run`. It starts the agent, waits for completion, exports its trace, and returns the matching reference. Returning before the agent finishes would let work outlive the SDK's concurrency limit

Return exactly one reference: `Run(trace={"session.id": session_id})` or `Run(trace={"trace_id": trace_id})`. These IDs come from your agent, not from the dataset. The SDK does not create agent sessions or instrument your application

Export these root-span attributes

| Attribute | Value |
| --- | --- |
| `session.id` | The session ID used by the completed trial |
| `agent.name` | Same value as `[tool.lens].project` |
| `agent.version` | Same version as the eval run |
| `deployment.environment` | `lens-eval` |

The [complete gateway example below](#complete-live-gateway-example) implements a real HTTP model call and OTLP export with no undefined helper functions. A deployed agent adapter must additionally handle that agent's authentication, follow-up messages, completion polling, and eval sandbox behavior

Set `LENS_BASE_URL` to the Lens server and `LENS_API_KEY` to a key allowed to access Lens routes. The environment URL overrides `base_url` in `pyproject.toml`. An inference-only key cannot download Lens dataset revisions

## Scorers and gates

Replace the quickstart's `evaluation` declaration with this configuration to request multiple scorers and thresholds; `task` remains the function defined above

```python
from datetime import timedelta

from lens import judge


evaluation: Final = Eval(
    "release-check",
    task=task,
    data="demo@1",
    scores=[
        scorers.task_completed(),
        scorers.called_before("run_tests", "open_pr"),
        judge("Did the agent satisfy the user's request using the available evidence?"),
    ],
    trials=3,
    concurrency=4,
    timeout_per_trial=timedelta(minutes=10),
    gate=Gate(
        regressions=0,
        critical=0,
        pass_rate=0.9,
        cost_per_case=0.50,
        min={"task_completed": 0.95},
    ),
)
```

Lens runs the scorers and computes the verdict. The SDK sends this configuration and displays the returned `Summary`. The local server uses its synthetic rule for every scorer, so it cannot validate judge quality or tool ordering

| Scorer | Request |
| --- | --- |
| `scorers.task_completed()` | Whether the task completed successfully |
| `scorers.called_before(first, then)` | Whether one named tool was called before another |
| `judge(prompt, model="")` | A judge with your rubric; an empty model lets the server select it |

| Gate field | Meaning |
| --- | --- |
| `regressions`, `critical` | Maximum regression counts, both default to 0 |
| `pass_rate` | Minimum fraction of passing cases |
| `cost_per_case` | Maximum server-reported cost per case in USD |
| `min` | Minimum aggregate score by declared scorer kind |

Scalar thresholds set to `None` are disabled. Trial counts are 1 through 10, default 1. Concurrency defaults to 8 and timeout to 20 minutes per trial. The contract combines repeated trials by majority; ties and missing trials fail

## Run from Python and inspect the result

Save this as `run_eval.py` alongside the quickstart's `evals/` directory

```python
from typing import Final

from evals.demo import evaluation


report: Final = evaluation.run()
print(f"{report.passed}/{report.total} cases passed")
print(f"Pass rate: {report.pass_rate:.0%}")
print(f"Cost per case: ${report.cost_per_case:.4f}")
print(f"Regressions: {len(report.regressions)}")
print(report.scores)
print(report.url)
report.assert_passed()
```

Run `uv run python run_eval.py` with the same environment as the quickstart. `report.passed` is a count; `report.gate.passed` is the boolean verdict. `assert_passed()` raises `lens.errors.GateFailed`, an `AssertionError`, if the gate fails

For an async application, the complete equivalent is

```python
import asyncio
from typing import Final

from evals.demo import evaluation


async def main() -> None:
    report: Final = await evaluation.arun()
    print(report.summary.model_dump_json(indent=2))
    report.assert_passed()


asyncio.run(main())
```

Inside an existing event loop, call `await evaluation.arun()` directly. The report also exposes `errors`, `baseline_run_id`, `baseline_version`, `fixed`, `regressions`, `gate`, and the raw `run` and `summary`. Version 1 does not return every case's score; use the server's report and regression links for investigation

A task exception or timeout is uploaded as a failed trial. Configuration and transport failures raise `lens.errors.LensError`; `ApiFailure` includes `status` and `code`. A failed gate returns a report normally unless you call `assert_passed()`

## Run a subset or change a gate

These expressions return new evals and leave the original unchanged. These IDs exist in the demo dataset; use the corresponding IDs from your own dataset

```python
selected: Final = evaluation.subset(case_ids=["case-0", "case-1"])
from_finding: Final = evaluation.subset(finding="1")
strict: Final = evaluation.gate(Gate(regressions=0, pass_rate=1))
```

A subset gets its own eval name and baseline. Contract v1 compares against compatible completed runs on `main`; other baseline branch names are rejected. An initial run has no comparable baseline, so use an absolute gate such as `pass_rate` when you need the first run to enforce a minimum

## CLI

The CLI discovers module-level `Eval` objects in `evals/`, or in the supplied file or directory

```bash
uv run lens eval
uv run lens eval evals/demo.py --eval demo
uv run lens eval --json > lens-runs.json
```

| Exit code | Meaning |
| --- | --- |
| `0` | Every gate passed |
| `1` | At least one gate failed |
| `2` | Configuration or infrastructure error |

JSON stdout contains `{"runs": [...]}` with the full run payloads. Task output goes to stderr. Completed runs are retained if another eval encounters an infrastructure error

To scaffold a new eval and workflow in your agent project

```bash
uv run lens init release-cases@7 --project my-agent
```

Replace `release-cases@7` with a real dataset. This creates an eval module, adds `[tool.lens]` configuration when needed, and writes `.github/workflows/lens.yml`. It refuses to overwrite files. Implement the generated task and add your agent's installation/startup steps before running it

## GitHub Actions

The generated workflow runs evals on `main` and same-repository PRs. Configure the repository secret `LENS_API_KEY` and variable `LENS_BASE_URL`. Your agent project must contain its eval files and `[tool.lens].project`

```yaml
name: Lens
on:
  push:
    branches: [main]
  pull_request:
permissions:
  contents: read
  checks: write
  pull-requests: write
jobs:
  eval:
    if: github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262
      - uses: actions/setup-python@a26af69be951a213d495a4c3e4e4022e16d87065
        with:
          python-version: '3.11'
      - run: python -m pip install -e .
      - uses: BerriAI/lens/packages/sdk/action@63be70d6b299bb22a0912da48bf703b14e886c95
        id: lens
        with:
          api-key: ${{ secrets.LENS_API_KEY }}
          base-url: ${{ vars.LENS_BASE_URL }}
          path: evals/
```

The Action installs the SDK, runs the evals, publishes a GitHub check and updates its PR comment, then enforces the gate exit code. Outputs are `passed` and `run-urls` (a JSON array). `github-token` defaults to the workflow token

The Action reference above is a pinned preview commit containing the working SDK. `lens init --action-ref owner/repository/path@commit` selects another reviewed commit. The standalone `berriai/lens-action@v1` has not been published. Since this repository is private, the consuming repository must have access to use its Action

A PR with no baseline receives a neutral Lens check. The Action still fails if an absolute gate fails. Fork PRs are skipped because secrets are unavailable

Create and result HTTP requests retry with bounded backoff. Create retries reuse an idempotency key; CI identities include the run ID, attempt, commit, and configuration. An interrupted job can still rerun agent work, so adapters with external side effects need their own deduplication

## Complete live gateway example

This is the full [live_gateway.py](examples/live_gateway.py) used for deployment verification. Save it as `evals/live_gateway.py` and run it with the settings below. It makes one real model call and exports one real trace per trial. It uses only `case.input`; a multi-turn agent adapter must also handle `case.followups`

```python
import json
import os
import time
from typing import Final
from uuid import uuid4

import httpx
from pydantic import BaseModel, ConfigDict

from lens import Case, Eval, Gate, Run, scorers


class Message(BaseModel):
    model_config = ConfigDict(frozen=True, extra="ignore")
    content: str


class Choice(BaseModel):
    model_config = ConfigDict(frozen=True, extra="ignore")
    message: Message


class Completion(BaseModel):
    model_config = ConfigDict(frozen=True, extra="ignore")
    id: str
    choices: tuple[Choice, ...]


async def task(case: Case) -> Run:
    session: Final = f"pass-sdk-smoke-{uuid4().hex}"
    trace_id: Final = uuid4().hex
    started: Final = time.time_ns()
    model: Final = os.environ["LENS_SMOKE_MODEL"]
    async with httpx.AsyncClient(timeout=120) as http:
        response: Final = await http.post(
            os.environ["GATEWAY_BASE_URL"].rstrip("/") + "/v1/chat/completions",
            headers={"Authorization": f"Bearer {os.environ['GATEWAY_API_KEY']}"},
            json={
                "model": model,
                "messages": [
                    {"role": "system", "content": "Write a brief response to this support request. Do not use tools."},
                    {"role": "user", "content": case.input},
                ],
                "max_completion_tokens": 512,
            },
        )
        response.raise_for_status()
        completion: Final = Completion.model_validate_json(response.content)
        if not completion.choices or not completion.choices[0].message.content.strip():
            raise ValueError("The gateway returned no response text")
        attributes: Final = {
            "session.id": session,
            "agent.name": "lens-sdk-smoke",
            "agent.version": os.environ["LENS_VERSION"],
            "deployment.environment": "lens-eval",
            "gen_ai.request.model": model,
            "gen_ai.response.id": completion.id,
            "gen_ai.input.messages": json.dumps([{"role": "user", "content": case.input}]),
            "gen_ai.output.messages": json.dumps([completion.choices[0].message.model_dump()]),
        }
        exported: Final = await http.post(
            os.environ["LENS_TRACE_ENDPOINT"],
            headers={"Authorization": f"Bearer {os.environ['LENS_TRACE_API_KEY']}"},
            json={
                "resourceSpans": [
                    {
                        "resource": {
                            "attributes": [{"key": "service.name", "value": {"stringValue": "lens-sdk-smoke"}}]
                        },
                        "scopeSpans": [
                            {
                                "scope": {"name": "lens-sdk-smoke"},
                                "spans": [
                                    {
                                        "traceId": trace_id,
                                        "spanId": uuid4().hex[:16],
                                        "name": "lens-sdk-live-trial",
                                        "kind": 1,
                                        "startTimeUnixNano": str(started),
                                        "endTimeUnixNano": str(time.time_ns()),
                                        "status": {"code": 1},
                                        "attributes": [
                                            {"key": key, "value": {"stringValue": value}}
                                            for key, value in attributes.items()
                                        ],
                                    }
                                ],
                            }
                        ],
                    }
                ],
            },
        )
        exported.raise_for_status()
    cost_header: Final = response.headers.get("x-litellm-response-cost")
    cost: Final = float(cost_header) if cost_header else None
    print(
        json.dumps(
            {
                "case_id": case.id,
                "trace_id": trace_id,
                "session_id": session,
                "model": model,
                "inference_status": response.status_code,
                "ingest_status": exported.status_code,
                "cost_usd": cost,
                "response_characters": len(completion.choices[0].message.content),
            }
        )
    )
    return Run(trace={"session.id": session}, cost_usd=cost)


evaluation: Final = Eval(
    "sdk-live-smoke",
    task=task,
    data=os.environ.get("LENS_SMOKE_DATASET", "demo@1"),
    scores=[scorers.task_completed()],
    trials=3,
    gate=Gate(pass_rate=1),
)
```

Use `[tool.lens].project = "lens-sdk-smoke"` for this example. Supply these values from your deployment

| Environment variable | Source |
| --- | --- |
| `GATEWAY_BASE_URL` | Gateway URL serving `/v1/chat/completions` |
| `GATEWAY_API_KEY` | Credential permitted to call the selected model |
| `LENS_TRACE_ENDPOINT` | Full OTLP HTTP trace ingest URL, including its route |
| `LENS_TRACE_API_KEY` | Credential allowed to ingest traces |
| `LENS_SMOKE_MODEL` | A model route configured on that gateway |
| `LENS_VERSION` | Eval version matching the trace's `agent.version` |
| `LENS_SMOKE_DATASET` | Dataset name and revision, default `demo@1` |
| `LENS_BASE_URL`, `LENS_API_KEY` | Eval API URL and credential; use the local server until the eval API is deployed |

Keep `LENS_VERSION` and `LENS_BRANCH` set when running outside a Git checkout. With the local server running, `LENS_BASE_URL=http://127.0.0.1:8765` and `LENS_API_KEY=lens-dev`, run

```bash
uv run lens eval evals/live_gateway.py --json
```

To use an exported real dataset locally, start the server with `uv run lens dev-server --dataset-file /absolute/path/cases.json`. The file must contain the dataset API's `EvalCases` response. The local alias remains `demo`; select `demo@N` where `N` is the revision in the file

The live example's session IDs deliberately contain `pass` for the local scorer. Its passing gate proves orchestration and transport. It does not establish answer quality

## What's built and what's pending

| Component | Current state |
| --- | --- |
| SDK and CLI | Dataset resolution, trial execution, bounded concurrency, timeouts, result uploads, retries, polling, reports, subsets and gates |
| Local server | Synthetic scoring, main baselines, regressions and gate calculation over in-memory runs |
| GitHub Action | Check and PR comment reporting, gate exit codes, outputs and workflow scaffolding |
| Real deployment verification | One real dataset case, three model calls, three OTLP uploads; all requests succeeded |
| Production eval backend | Pending Rust eval lifecycle, scoring, baseline selection and Runs UI integration |
| Agent integration | Pending the deployed agent's service-token/sandbox adapter and full regression PR test |
| Publishing | Python package and standalone Action release pending |

The SDK suite has 50 passing behavioral tests with 92% statement coverage. A focused mutation run caught 20 of 20 injected faults. The live smoke was repeated from a built wheel in a clean environment. Detailed evidence and remaining limits are in [verification.md](validation/verification.md)

### Contract handoff for Ishaan

The accepted package name is `lens-evals`, with `import lens`. Rust's `lens-contract` crate owns `schema/lens.v1.json`; the SDK generates its Pydantic wire models with pinned `datamodel-code-generator`. The HTTP routes, request/response bodies, status/error codes, `X-Lens-Contract: 1` header, and Summary semantics remain the agreed v1 contract

The canonical schema is not in this checkout yet. CI explicitly uses the provisional appendix schema and switches to the canonical path when it appears. Shared fixtures are expected at `runtime/crates/contract/fixtures/lens_eval/`; tests consume that path when present, otherwise the SDK's local copies. Both languages still need to validate the shared fixtures together

Once the Rust schema and eval endpoints land, regenerate the models, run the shared fixtures, and repeat the lifecycle against the deployed eval API. The SDK already implements create, per-trial result upload, finish, polling, and baseline-report retrieval

## SDK development

Run these from a Lens checkout

```bash
uv sync --project packages/sdk --frozen
uv run --project packages/sdk pytest packages/sdk/tests --cov=lens --cov-config=packages/sdk/pyproject.toml
uv run --project packages/sdk mypy packages/sdk/src/lens packages/sdk/scripts/generate_contract.py
uv run --project packages/sdk ruff check packages/sdk
uv run --project packages/sdk ruff format --check packages/sdk
uv run --project packages/sdk python packages/sdk/scripts/mutation_check.py
uv build --project packages/sdk --out-dir packages/sdk/dist
```

Check generated models against the canonical Rust schema once it lands

```bash
uv run --project packages/sdk python packages/sdk/scripts/generate_contract.py --check
```

Until then, the explicit provisional check is

```bash
uv run --project packages/sdk python packages/sdk/scripts/generate_contract.py \
  --schema packages/sdk/tests/fixtures/appendix.v1.json --check
```
