# Developing Lens

This repository contains the Lens source extraction and the standalone implementation in progress. It is available for collaborative development. The shared UI builds, and the standalone API can serve it with live Rust ingestion and ClickHouse storage. A source-built standalone image is being qualified with ClickHouse. Published artifacts, complete lifecycle, migration and cross-repository qualification remain open

The target deployment is Lens with ClickHouse, including its configured Keeper component. The API runtime and Lens repositories use ClickHouse. PostgreSQL helpers remain for migration work, and their driver is an optional `migration` extra

## Repository layout

| Path | Responsibility |
| --- | --- |
| `src/litellm_lens/` | Legacy parity reference, pending removal after the Rust replacements qualify |
| `src/worker/` | Rust API, authentication, ingestion, state, investigations and sandbox (Python sandbox sources in `crates/lens/sandbox/`) |
| `src/ui/lib/` | Shared Lens React UI, owned here and hosted by standalone Lens and LiteLLM |
| `src/ui/app/` | Small standalone Next.js shell, exported as static files served by Rust |
| `deploy/runtime/` | Complete standalone image, including the static UI |
| `deploy/clickhouse/` | ClickHouse coordination configuration |
| `deploy/lens/` | Standalone Compose bundle, repeatable startup and container smoke tests |
| `migrations/legacy/` | Source PostgreSQL migrations retained for migration compatibility |
| `scripts/` | Schema and release tooling |
| `tests/` | Copied behavior tests and standalone integration tests |
| `docs/extraction/` | Completion contract, baseline provenance and qualification ledger |

Python unit tests mirror the package under `tests/unit/litellm_lens/`. Deployment scripts are being reorganized. No sibling checkout is required by the Rust workspace or Python package dependency resolution

## Preview and build the UI

Use Node.js 24.14.1 or newer and npm 11.10.0 or newer. From the repository root:

```sh
npm ci
npm run dev:ui
```

Open [the Lens preview](http://127.0.0.1:3100/ui/?demo=true). This uses the existing read-only demo data. For the live tracing path, see [standalone development startup](docs/extraction/standalone-startup.md). For the source-built container path, see [Run Lens](deploy/lens/README.md)

```sh
npm run typecheck:ui
npm run test:ui
npm run build:ui
npm run qualify:ui-package
```

The production build writes static files to `src/ui/app/out`. Package qualification packs `@litellm/lens-ui`, installs it into a temporary consumer with its own dependencies, and builds that consumer. It prints the package integrity and output path. See [the UI package](src/ui/lib/README.md) for the embedding boundary and [qualification notes](docs/extraction/ui-package.md) for the current evidence and limits

## Checks available now

Backend changes go in the Rust workspace, following [the migration specification](docs/rust-migration.md). The Python API remains a parity reference until its Rust replacements are qualified; do not extend it. The eval SDK and sandbox interpreter are the documented Python exceptions

The worker protocol types live in `src/worker/crates/contract/src/worker/`. After changing those types, regenerate the checked-in schema and UI declarations from the repository root:

```sh
npm run generate:worker-contract
npm run check:worker-contract
cargo test --manifest-path src/worker/Cargo.toml -p lens-contract -p litellm-lens
```

The worker protocol remains version 7. Its generated artifact is `schema/lens-worker.v7.json`; the public eval API uses `schema/lens.v1.json` and the separate contract version 1. Worker messages and public HTTP responses have different default-field requirements, so do not substitute their types indiscriminately. See [worker contract qualification](docs/extraction/rust-worker-contract.md)

Run the Rust checks from the repository root:

```sh
cargo fmt --manifest-path src/worker/Cargo.toml --all --check
cargo clippy --manifest-path src/worker/Cargo.toml --workspace --all-targets -- -D warnings
cargo test --locked --manifest-path src/worker/Cargo.toml
```

Live storage tests start an isolated ClickHouse container with Keeper configured. Docker must be running. To reuse a dedicated test server, set `CLICKHOUSE_STATE_TEST_URL`; the tests create and remove their own databases

To qualify the complete image and calculation sandbox on the host's native Linux architecture:

```sh
docker build --build-arg LENS_VERSION=local-test -f deploy/runtime/Dockerfile -t lens:local-test .
deploy/lens/smoke.sh lens:local-test local-test
docker build --target smoke --build-arg LENS_VERSION=local-test -f deploy/runtime/Dockerfile -t lens:smoke .
docker run --rm --read-only --cap-drop ALL --security-opt no-new-privileges --network none --tmpfs /tmp:rw,noexec,nosuid,size=1g lens:smoke
```

The image smoke test checks private setup, startup, UI delivery, API access and restart. It does not substitute for the full browser, provider, migration and recovery acceptance cases in the completion plan

## Completion and source provenance

Copied tests that still initialize the old gateway runtime need standalone fixtures. A full test-suite run is not yet qualified

The [completion plan](docs/extraction/completion-plan.md) includes the standalone, embedded, release, deployment, migration and ClickHouse requirements. The [implementation ledger](docs/extraction/implementation-state.json) records open requirements and partial evidence

The extraction source is LiteLLM commit `0721cffab2ecbde51cdef9aeca0ce1c16b3e0aa9`, fetched from main before implementation. Relevant Git history and baseline file hashes are preserved under `docs/extraction/`. Track new changes to Lens in LiteLLM until cutover
