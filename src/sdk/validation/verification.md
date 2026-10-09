# SDK verification, 2026-10-08

The Python API is now backed by `lens-evals-sdk` and `lens-evals-python` in `src/worker/crates/`. The package is `lens-evals==0.1.0a2`, under `src/sdk/`

## Native SDK

57 Rust behavior tests and 36 Python API/CLI tests pass locally. Python statement coverage is 92%; that percentage covers the Python boundary, not the Rust core. Strict Clippy, Rust formatting, Ruff, strict mypy, and provisional schema drift checks pass

The tests cover the 36-case, three-trial lifecycle; main baseline, individual regressions, revert and repeat; majority ties and missing trials; compatible baselines; server-authoritative gates; bounded concurrency; coroutine cancellation and cleanup; retries and lost acknowledgements; dataset identity; subsets; malformed responses; setup preservation; read-only diagnostics; GitHub comment ownership and verdicts; and CLI exit codes

Two regressions found during rehearsal have dedicated tests. Python cancellation must finish before a Rust execution slot is reused, including when coroutine cleanup itself awaits. Discovery must execute the current eval source after a same-size edit or revert, even with an unchanged timestamp

The focused mutation suite catches 14 of 14 deliberate faults in Rust execution, transport, setup, and reporting. [mutations.json](mutations.json) records the cases. This is targeted regression evidence, not exhaustive mutation coverage

```sh
cargo test --manifest-path src/worker/Cargo.toml -p lens-evals-sdk
uv run --project src/sdk pytest src/sdk/tests
uv run --project src/sdk python src/sdk/scripts/mutation_check.py
```

## Install and setup rehearsal

The source distribution built a native macOS arm64 wheel. In a fresh virtualenv and a fresh project, with Cargo removed from PATH, that wheel installed and ran `lens init --demo`, `lens doctor --json`, and the real `lens eval --json` CLI

The baseline completed 36 cases and 108 trials. Changing the demo trace prefix from `pass-` to `fail-` produced exit 1 and 36 regressions. Reverting produced exit 0, and a repeat of the same commit produced zero regressions. [onboarding.json](onboarding.json) records the results

Cached wheel installation plus that synthetic rehearsal took 1.70 seconds. It excludes downloading the wheel, connecting an actual agent, service authentication, and implementing the agent's eval mode. It does not establish the under-ten-minute promise for a new real agent

The SDK workflow builds Linux x86_64, macOS arm64/x86_64, and Windows x86_64 wheels, installs each with `--only-binary`, and exercises the public Python API. The Action job selects a separate virtualenv with an agent dependency and no pip, runs the actual composite Action, publishes a real GitHub check, and validates its outputs. See [PR #16 checks](https://github.com/BerriAI/lens/pull/16/checks) for the current commit's hosted results

The package has not been published to PyPI. Wheels are private CI artifacts. Source installs require Rust and access to this repository

## Earlier live deployment evidence

The previous Python implementation, `0.1.0a1`, was tested against `https://gateway-dev.litellm-sandbox.ai`. A temporary Lens-scoped key downloaded `model-release-bench@1`, dataset `9154d122-1f48-48d5-ac9c-5d622031b6e8`, containing one production-derived case

The live-gateway example called `openai/gpt-6.1-sol` three times and exported three completed OTLP traces. All inference and ingest requests returned HTTP 200, with total reported cost $0.004346. A wheel repeat at the same evaluation version reported zero regressions against a local synthetic contract server

[Open the persisted trace](https://gateway-dev.litellm-sandbox.ai/ui/lens?tab=traces&agent=lens-sdk-smoke&trace=53f57c88f2834c228e32b8493d44f5b3&span_tab=attributes&fullscreen=true)

![Recorded trace attributes](live-trace.jpg)

[live.json](live.json) preserves sanitized evidence from that earlier implementation. It is not a live validation of the new Rust transport. The temporary key was blocked and verified unusable, and downloaded case content was removed

## Remaining integration work

The deployed eval lifecycle/scorers, canonical Rust schema, and real-agent regression PR ship test remain unverified. The production API was absent at the earlier live check. Local server gates in these tests are synthetic

Ishaan owns the real lifecycle routes, trace scoring, baseline selection, comparison UI, and `lens-contract` schema. Once the schema lands, regenerate Pydantic models, replace the SDK's provisional Rust wire structs with the canonical types, and validate the shared fixtures under `src/worker/crates/contract/fixtures/lens_eval/`

The agent integration must await completed traces, run the PR's actual version, and implement its service-token/sandbox eval mode. Then run the actual regression PR, revert it, and repeat an unchanged commit. No SDK-only test can establish that server-and-agent result in advance
