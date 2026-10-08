# Developing Lens

This repository contains the Lens source extraction and the standalone implementation in progress. It is available for collaborative development. The shared UI package builds and the standalone UI can be previewed. It is not yet an installable standalone release: application startup, remaining ClickHouse repositories and cross-repository qualification remain open

The target deployment is Lens with ClickHouse, including its configured Keeper component. PostgreSQL code currently present is transitional extraction work and migration material, not the target runtime dependency

## Repository layout

| Path | Responsibility |
| --- | --- |
| `src/litellm_lens/` | Python API, investigation state and model access |
| `runtime/` | Rust ingestion, trace storage, investigation worker and sandbox |
| `packages/ui/` | Shared Lens React UI, owned here and hosted by standalone Lens and LiteLLM |
| `apps/web/` | Small standalone Next.js shell, exported as static files for the Python API |
| `deploy/runtime/` | Rust runtime image and Python sandbox packaging |
| `deploy/clickhouse/` | ClickHouse coordination configuration |
| `deploy/lens/` | Legacy installation assets being migrated to the standalone bundle |
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

Open [the Lens preview](http://127.0.0.1:3100/ui/?demo=true). This uses the existing read-only demo data. The live standalone API is still being completed; the preview does not demonstrate ingestion, persistence or provider-backed investigations

```sh
npm run typecheck:ui
npm run test:ui
npm run build:ui
npm run qualify:ui-package
```

The production build writes static files to `apps/web/out`. Package qualification packs `@litellm/lens-ui`, installs it into a temporary consumer with its own dependencies, and builds that consumer. It prints the package integrity and output path. See [the UI package](packages/ui/README.md) for the embedding boundary and [qualification notes](docs/extraction/ui-package.md) for the current evidence and limits

## Checks available now

Install Python dependencies with `uv sync --dev`. These copied core behavior tests pass in the extracted package:

```sh
uv run pytest tests/unit/litellm_lens/test_state.py tests/unit/litellm_lens/test_reviews.py tests/unit/litellm_lens/test_agent_contract.py tests/unit/litellm_lens/test_sources.py tests/unit/litellm_lens/test_datasets.py tests/unit/litellm_lens/test_signals.py -q
cargo check --manifest-path runtime/Cargo.toml
```

The ClickHouse state prototype, dataset repository and signal repository have integration tests against a real ClickHouse server with KeeperMap enabled. Start the isolated test stack and run:

```sh
docker compose -f tests/integration/clickhouse.compose.yaml up -d --wait
CLICKHOUSE_STATE_TEST_URL=http://127.0.0.1:18124 uv run pytest tests/integration/test_clickhouse_state.py tests/integration/database/test_lens_dataset_repository.py tests/integration/database/test_lens_signal_repository.py -q
```

The tests create and drop isolated databases. Dataset revisions and their latest summaries publish atomically. Signal claims and result writes retain their lease and configuration checks. Investigation records and sessions still need their ClickHouse adapters, and full capacity, migration and recovery qualification remains open. These tests do not demonstrate that the full Lens product runs end to end

## Completion and source provenance

The [completion plan](docs/extraction/completion-plan.md) includes the standalone, embedded, release, deployment, migration and ClickHouse requirements. The [implementation ledger](docs/extraction/implementation-state.json) records open requirements and partial evidence

The extraction source is LiteLLM commit `0721cffab2ecbde51cdef9aeca0ce1c16b3e0aa9`, fetched from main before implementation. Relevant Git history and baseline file hashes are preserved under `docs/extraction/`. Track new changes to Lens in LiteLLM until cutover
