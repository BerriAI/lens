# SDK verification, 2026-10-08

The Python API is backed by `lens-evals-sdk` and `lens-evals-python` in `src/worker/crates/`. The package is under `src/sdk/`. The current follow-up is `lens-evals==0.1.0a3`

## Earlier contract alignment, 0.1.0a3

86 Rust behavior tests and 38 Python API/CLI tests pass locally. Python boundary coverage is 92%. Strict Clippy, formatting, Ruff, strict mypy, and generated-model drift checks pass

The added tests exercise returning an accepted trace before completion, root-span closure, 120-second idle closure, custom per-trial deadlines sent over HTTP, late or missing traces, independently stamped build versions, metadata finding selection, and failed absolute gates without a baseline. The installer tests reject a tampered wheel before installation and reuse an already installed native package. The focused mutation suite catches all 19 deliberate faults; [mutations.json](mutations.json) records the results

This version sends `timeout_per_trial_ms` and consumes `DatasetCase.meta["finding_id"]`. At this earlier checkpoint, the canonical eval schema and deployed lifecycle were pending and local scoring was synthetic. The merged qualification above supersedes that integration status

## Previous native SDK, 0.1.0a2

57 Rust behavior tests and 36 Python API/CLI tests pass locally. Python statement coverage is 92%; that percentage covers the Python boundary, not the Rust core. Strict Clippy, Rust formatting, Ruff, strict mypy, and provisional schema drift checks pass

The tests cover the 36-case, three-trial lifecycle; main baseline, individual regressions, revert and repeat; majority ties and missing trials; compatible baselines; server-authoritative gates; bounded concurrency; coroutine cancellation and cleanup; retries and lost acknowledgements; dataset identity; subsets; malformed responses; setup preservation; read-only diagnostics; GitHub comment ownership and verdicts; and CLI exit codes

Two regressions found during rehearsal have dedicated tests. Python cancellation must finish before a Rust execution slot is reused, including when coroutine cleanup itself awaits. Discovery must execute the current eval source after a same-size edit or revert, even with an unchanged timestamp

The a2 focused mutation suite caught 14 of 14 deliberate faults in Rust execution, transport, setup, and reporting. The current result file includes the additional a3 cases. This is targeted regression evidence, not exhaustive mutation coverage

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

The a2 package was not published to PyPI. Its wheels were private CI artifacts. Source installs require Rust and access to this repository

## Native live deployment evidence, 0.1.0a2

The a2 Rust client resolved and downloaded the real `model-release-bench@1` dataset from gateway-dev. The installed native wheel made three real `openai/gpt-6.1-sol` calls and uploaded completed traces, all HTTP 200. Reported cost for the successful run was $0.005726. A repeat at the same version executed three new trials, with zero errors and zero synthetic regressions

[Open the persisted native trace](https://gateway-dev.litellm-sandbox.ai/ui/lens?tab=traces&agent=lens-sdk-smoke&trace=beb1e8681576420787c0da412ac68509&span_tab=attributes&fullscreen=true)

[native-live.json](native-live.json) records sanitized evidence. Dataset access, inference, and trace ingestion were real. Eval lifecycle and gates used a local synthetic server because the deployed eval API returned 404. Both temporary credentials were revoked or blocked and verified unusable

This is a2 evidence, not a live production validation of a3's contract additions

## Earlier live deployment evidence, 0.1.0a1

The previous Python implementation, `0.1.0a1`, was tested against `https://gateway-dev.litellm-sandbox.ai`. A temporary Lens-scoped key downloaded `model-release-bench@1`, dataset `9154d122-1f48-48d5-ac9c-5d622031b6e8`, containing one production-derived case

The live-gateway example called `openai/gpt-6.1-sol` three times and exported three completed OTLP traces. All inference and ingest requests returned HTTP 200, with total reported cost $0.004346. A wheel repeat at the same evaluation version reported zero regressions against a local synthetic contract server

[Open the persisted trace](https://gateway-dev.litellm-sandbox.ai/ui/lens?tab=traces&agent=lens-sdk-smoke&trace=53f57c88f2834c228e32b8493d44f5b3&span_tab=attributes&fullscreen=true)

![Recorded trace attributes](live-trace.jpg)

[live.json](live.json) preserves sanitized evidence from that earlier implementation. It is not a live validation of the new Rust transport. The temporary key was blocked and verified unusable, and downloaded case content was removed

## Remaining release and integration qualification

The canonical schema and lifecycle implementations are integrated. Production rollout, published SDK wheel qualification against the final service image, and the real-agent regression PR ship test remain separate acceptance checks. Older hosted deployments that still return 404 for the eval API need a compatible Lens release before they can serve these SDK calls

An agent must return its accepted trace reference, stamp its own build SHA, and implement its service-token or sandbox eval mode. The assembled service tests and real main/regression/fix/repeat run now exercise server closure and gate behavior. They do not by themselves prove every external agent integration or a reviewed regression PR being merged and deployed
