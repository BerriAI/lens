# Lens CLI

This directory owns the `lens` commands, local CSV eval runner, checkpoints and terminal output

The Rust crate is a member of the workspace in `../worker`. `src/` contains command parsing, setup, the local runner and terminal views. `python/lens_cli/` loads Python eval configs and calls the native implementation. `tests/` contains the Rust and Python CLI tests, and `examples/local_eval/` contains the runnable CSV example

Install the existing `lens-evals` package from `../sdk`. Its wheel includes the CLI adapters and native bindings, so the `lens` command, `python -m lens.cli` and public `lens` imports keep working. The build script copies the Python adapters into Cargo's output directory for Maturin to include in both regular and editable wheels

From the repository root:

```bash
uv sync --project src/sdk --frozen
uv run --project src/sdk lens --help
uv run --project src/sdk lens demo --fast
cargo test --manifest-path src/worker/Cargo.toml -p lens-cli -p lens-evals-sdk
uv run --project src/sdk pytest -c src/sdk/pyproject.toml src/sdk/tests src/cli/tests/python
```

See the [local eval example](examples/local_eval/README.md) for `lens run`, `lens resume` and `lens show`

## LiteLLM SDK configs

Declare `targets = [litellm.EvalTarget(...)]` and either `tasks = [litellm.EvalTask(...)]` or provide a CSV. Run config-defined tasks with `lens run --config evals.py`; CSV runs use `lens run tasks.csv --config evals.py`. `--trials N` repeats each task against each target using a fresh SDK attempt

CSV rows become SDK tasks with a case-insensitive expected-text `PredicateCheck` by default. Rows without `expected` need an explicit `task(row)` factory. An optional `task(row)` function returns an `EvalTask` to supply a custom check, files, setup commands or workspace. Its row contains `id`, `prompt`, `expected` and the extra CSV columns, and the returned task must preserve the row's id

The optional `run_options` mapping accepts the SDK attempt's `max_turns`, `timeout`, `setup_timeout`, `api_base` and `api_key` options. Read credentials from the environment. Lens saves neither connection option values nor SDK target options in its run metadata

Install with `uv sync --project src/sdk --extra agent-evals --frozen` to use the pinned LiteLLM source change exposing `aagent_eval_attempt`; earlier released LiteLLM packages do not provide it. Each SDK harness needs its own runtime installed. The [example](examples/local_eval/README.md) covers the Python tool loop and Pi. Existing `lens.Target(..., run=callback)` configs continue to work without LiteLLM installed

Lens owns scheduling, progress and resumable checkpoints. The SDK owns agent execution, tools, sandboxes and checks. A failed check exits 1; an agent or judge error exits 2. Saved results retain SDK stop reasons and separate agent/judge usage and costs, with unavailable metrics marked unknown

Resume checks the config source, resolved target options and tool definitions, execution options and task snapshots. Credential rotation does not change the fingerprint. Imported implementations outside those definitions, workspace contents and remote model deployments can still change independently; start a new run after changing them
