# Developing Lens

This branch contains the complete Lens source extraction and the standalone implementation in progress. It is available for collaborative development. It is not yet an installable standalone release: application startup, the UI package build, ClickHouse repository integration and cross-repository qualification remain open

The target deployment is Lens with ClickHouse, including its configured Keeper component. PostgreSQL code currently present is transitional extraction work and migration material, not the target runtime dependency

## Repository layout

| Path | Responsibility |
| --- | --- |
| `src/litellm_lens/` | Python API, investigation state and model access |
| `runtime/` | Rust ingestion, trace storage, investigation worker and sandbox |
| `packages/ui/` | Shared Lens React UI, owned here and hosted by standalone Lens and LiteLLM |
| `deploy/` | Installation and image assets being adapted for standalone Lens |
| `migrations/legacy/` | Source PostgreSQL migrations retained for migration compatibility |
| `scripts/` | Schema and release tooling |
| `tests/` | Copied behavior tests and standalone integration tests |
| `docs/extraction/` | Completion contract, baseline provenance and qualification ledger |

The copied test paths and deployment scripts are being reorganized. No sibling checkout is required by the Rust workspace or Python package dependency resolution

## Checks available now

Install Python dependencies with `uv sync --dev`. These copied core behavior tests pass in the extracted package:

```sh
uv run pytest tests/unit/proxy/lens/test_state.py tests/unit/proxy/lens/test_reviews.py tests/unit/proxy/lens/test_agent_contract.py tests/unit/proxy/lens/test_sources.py tests/unit/proxy/lens/test_datasets.py -q
cargo check --manifest-path runtime/Cargo.toml
```

The ClickHouse state prototype has integration tests against a real ClickHouse server with KeeperMap enabled. Set `CLICKHOUSE_STATE_TEST_URL` to an isolated local test instance, then run:

```sh
uv run pytest tests/integration/test_clickhouse_state.py -q
```

The test creates and drops isolated databases. The storage prototype tests are not evidence that the full Lens product already runs end to end

## Completion and source provenance

The [completion plan](docs/extraction/completion-plan.md) includes the standalone, embedded, release, deployment, migration and ClickHouse requirements. The [implementation ledger](docs/extraction/implementation-state.json) records open requirements and partial evidence

The extraction source is LiteLLM commit `0721cffab2ecbde51cdef9aeca0ce1c16b3e0aa9`, fetched from main before implementation. Relevant Git history and baseline file hashes are preserved under `docs/extraction/`. Track new changes to Lens in LiteLLM until cutover
