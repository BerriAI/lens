# Local eval example

Run the same support-ticket tasks with the LiteLLM SDK's tool loop and Pi harness. Both use `EvalTarget`, `EvalTask` and `PredicateCheck`; Lens schedules one `aagent_eval_attempt` per task, target and trial and saves its result

This example requires the LiteLLM source revision that introduces the public `aagent_eval_attempt` API. It is not available in earlier published packages. The `agent-evals` extra pins the companion source revision. Lens needs Python 3.11+, `uv` and Rust 1.99.0 when building from source

From the Lens repository root:

```bash
uv sync --project src/sdk --extra agent-evals --frozen
```

The `tool_loop` harness runs in Python. Pi also needs Node.js and its CLI on `PATH`; the SDK adapter supports `@earendil-works/pi-coding-agent` 1.3.0. Install that runtime separately if using the Pi target

Configure the SDK's gateway connection using `LITELLM_PROXY_API_BASE` and `LITELLM_PROXY_API_KEY`. Set `LENS_MODEL_A` and `LENS_MODEL_B` to your gateway's model names with the `litellm_proxy/` prefix. The defaults are `litellm_proxy/model-a` and `litellm_proxy/model-b`. Keep credential values in your environment

```bash
cd src/cli/examples/local_eval
uv run --project ../../../sdk --extra agent-evals lens run tasks.csv --config evals.py
```

The CSV contains 20 tickets and three targets. One trial makes 60 agent attempts and incurs model charges. Eight tickets require the shared `lookup_ticket` tool. `task(row)` constructs each SDK task and its predicate check; it can also set SDK `workspace`, `files`, `setup`, `CommandCheck` or `JudgeCheck` values

A config can define SDK tasks directly, without a CSV:

```bash
uv run --project ../../../sdk --extra agent-evals lens run --config native_evals.py --trials 2
```

Use `--only tool_loop:litellm_proxy/model-a` to select one target, or `--fast` to print the report without live progress

Results are saved under `.lens/runs/<run_id>/`. Resume unfinished attempts, or print a saved report:

```bash
uv run --project ../../../sdk --extra agent-evals lens resume <run_id>
uv run --project ../../../sdk --extra agent-evals lens show
```

A passing run exits 0, failed checks exit 1, and execution or judging errors exit 2. Checkpoints retain the SDK stop reason, check detail, usage and separate agent/judge costs. Missing metrics stay unknown. A changed config or task snapshot is rejected on resume
