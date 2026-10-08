# Lens evals SDK

`lens-evals` is the Python distribution. Application code imports `lens`. It runs your async task against a pinned Lens dataset revision, submits trace references, and displays the server's scores and gate verdict

This preview implements the SDK, CLI, development contract server, and composite GitHub Action. It is isolated from the production backend. The Rust eval API and canonical Rust-exported schema are still pending, so production scoring and the full agent regression ship test remain integration work

Install the preview from this checkout with `uv add --dev ./packages/sdk`. Once published, the command will be `uv add --dev lens-evals`. Python 3.11 or newer is required. The SDK can coexist with the existing `litellm-lens` distribution because they use different import packages

```python
from lens import Case, Eval, Gate, Run, scorers


async def task(case: Case) -> Run:
    session = await my_agent.run(case.input, followups=case.followups, metadata=case.meta)
    await session.wait_until_complete()
    return Run(trace={"session.id": session.id}, cost_usd=session.cost_usd)


evaluation = Eval(
    "regressions",
    task=task,
    data="production-cases@7",
    scores=[scorers.task_completed(), scorers.called_before("run_tests", "open_pr")],
    trials=3,
    gate=Gate(regressions=0, critical=0, pass_rate=0.9),
)
```

The task adapter belongs to your agent. It must await completion to keep live agent sessions within the configured concurrency limit. It must export `session.id`, `agent.name`, `agent.version`, and `deployment.environment=lens-eval` on its root span. Agent-specific auth, sandboxing, and disabling external side effects belong in that adapter and agent deployment

Put evaluations in module-level variables under `evals/`, and configure the agent name

```toml
[tool.lens]
project = "my-agent"
evals = "evals/"
base_url = "https://your-lens-gateway.example"
```

Set `LENS_API_KEY` to a key authorized for Lens routes. `LENS_BASE_URL` overrides the configured URL. Inference-only virtual keys cannot download dataset revisions

```bash
lens init production-cases@7 --project my-agent
lens eval
lens eval evals/regressions.py --eval regressions
lens eval --json
```

`init` creates an eval scaffold, adds configuration when needed, and creates `.github/workflows/lens.yml`. It refuses to overwrite files. Implement the task and adjust the workflow's agent dependency/startup steps before using it. The scaffold uses a strict pass-rate gate so an unimplemented task fails even without a baseline

The generated workflow currently references the preview branch Action in this repository. Use `--action-ref owner/repository/path@commit` to pin a reviewed commit. The intended standalone `berriai/lens-action@v1` has not been published

`lens eval` exits 0 when all gates pass, 1 when any gate fails, and 2 for configuration or infrastructure errors. JSON stdout contains only `{"runs": [...]}`. Task/module stdout is redirected to stderr. If one eval fails at the infrastructure layer, completed runs are retained in JSON and the command exits 2. The terminal table reports task submissions and errors. Per-case scoring is available through the server's report links and regression summaries, since v1 does not return every case score

For Python callers, `evaluation.run()` is synchronous and `await evaluation.arun()` is asynchronous. Both return `Report`, with all Summary fields, `.url`, `.summary`, and `.assert_passed()`. A failed gate raises `GateFailed`, an `AssertionError` subclass, only when explicitly asserted. A task exception or timeout is uploaded as a trial error. HTTP failures remain infrastructure errors

`.subset(case_ids=[...])`, `.subset(finding=...)`, and `.gate(Gate(...))` return copies. Subsets get distinct eval identities to keep their baselines separate. Dataset names without a revision are resolved once per run and pinned. A legacy dataset-list fallback supports deployments that do not yet expose `/lens/datasets/resolve`

Contract v1 selects the `main` baseline. Other baseline branch names are rejected until the server contract can express them. Gates are sent to Lens and never recomputed by the SDK. A PR with no baseline has a neutral Lens check, including when an absolute threshold fails; the CLI and Action still enforce `gate.passed` with exit 1

The Action accepts `api-key`, `base-url`, `path`, and optional `github-token`, and returns `passed` and `run-urls`. Give its token `checks: write` and `pull-requests: write`. Comments are upserted by eval marker only when owned by `github-actions[bot]`. Fork PRs are skipped by the generated workflow because secrets are unavailable. Never run untrusted PR code with secrets through `pull_request_target`

HTTP create/result requests retry with bounded exponential backoff. Create retries reuse the same idempotency key. CI identities include the workflow run ID, attempt, commit and evaluation configuration. This guarantees request deduplication, not exactly-once agent execution after a crashed job. Tasks with external side effects must provide their own deduplication

## Development and verification

```bash
uv sync --project packages/sdk --frozen
uv run --project packages/sdk pytest packages/sdk/tests
uv run --project packages/sdk mypy packages/sdk/src/lens packages/sdk/scripts/generate_contract.py
uv run --project packages/sdk ruff check packages/sdk
uv run --project packages/sdk python packages/sdk/scripts/mutation_check.py
uv build --project packages/sdk --out-dir packages/sdk/dist
```

Run the loopback-only development server in one terminal

```bash
uv run --project packages/sdk lens dev-server
```

In a project with `[tool.lens].project = "demo"`, run the toy example with `LENS_BASE_URL=http://127.0.0.1:8765` and `LENS_API_KEY=lens-dev`. The dev server accepts `--dataset-file` containing the existing `EvalCases` JSON response. Its dataset alias is `demo`

The development server intentionally uses synthetic scoring: trace references containing `pass` pass. Missing/error trials fail, ties fail, and compatible completed main runs become baselines. It does not inspect traces, run judges, infer gateway spend, or emulate trace-arrival timeouts. Its authenticated run URLs return JSON, not a Runs UI. Validation of malformed requests uses ordinary FastAPI errors where v1 defines no ApiError code

`examples/live_gateway.py` calls a real gateway model and exports a real OTLP trace for every task. Set `GATEWAY_BASE_URL`, `GATEWAY_API_KEY`, `LENS_TRACE_ENDPOINT`, `LENS_TRACE_API_KEY`, `LENS_SMOKE_MODEL`, and `LENS_VERSION`; set `[tool.lens].project="lens-sdk-smoke"`. Its `pass-sdk-smoke-...` session IDs are intentionally compatible with the stub. A passing stub result proves transport and orchestration, not the quality of that model's answer

The canonical generation command is

```bash
uv run --project packages/sdk python packages/sdk/scripts/generate_contract.py --check
```

It consumes `schema/lens.v1.json` exported by the Rust contract crate and fails when that file is absent. Until it lands, `_generated.py` is generated from the supplied appendix's provisional JSON schema with pinned `datamodel-code-generator`. CI explicitly reports this fallback and will switch to the canonical path as soon as it exists

```bash
uv run --project packages/sdk python packages/sdk/scripts/generate_contract.py \
  --schema packages/sdk/tests/fixtures/appendix.v1.json --check
```

Golden fixtures currently live under `tests/fixtures/lens_eval/`. Tests consume `runtime/crates/contract/fixtures/lens_eval/` when present, validate the same payloads against generated models and the original appendix, and compare the 36-by-3 lifecycle summaries. Both Rust and Python still need to validate those fixtures together once Ishaan's crate lands

See `validation/verification.md` for observed deployment evidence and remaining integration limits
