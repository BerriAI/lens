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
