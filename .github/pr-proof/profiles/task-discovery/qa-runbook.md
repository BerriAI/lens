# Task discovery QA runbook

Tested commit `dc0a714f5f63d5b960141656ac112b02f05378b9` on `joshua-berri/lens:litellm_lens_task_profiles`, compared with `main` at `5109b59b5575a00d752a8dd3134211c573560069`. Later commits on the branch only add this runbook and its screenshots. The images in the parent folder belong to an earlier pull request and are not evidence for this one

## Reproduce

```bash
git checkout litellm_lens_task_profiles
bash deploy/profiles-demo/demo.sh             # builds Lens for this commit, LiteLLM on 4400, Lens on 4320, seeds sample data
bash deploy/profiles-demo/demo.sh password    # LiteLLM admin password
bash deploy/profiles-demo/demo.sh lens-token  # Lens admin token
```

The seeded sample data needs no provider keys and makes no model calls. A live discovery needs three aliases on the gateway in `deploy/profiles-demo/litellm-config.yaml`, with their keys in `deploy/profiles-demo/.env`: a chat model, a model reported with `mode: embedding`, and a Jev model (`typesafe` or `mode: decisions`). The run below used Claude Haiku, a Qwen3 embedding deployment and `typesafe/jev-latest`

Open `http://localhost:4400/ui`, choose the Lens tab, then Profiles, and pick an agent. **Configure task discovery** in the page header opens the settings panel. Count model calls in the LiteLLM spend logs by the `litellm-lens-profiles` request tag

## What was checked at the tested commit

These are the steps of the live run, through the LiteLLM gateway, against an instrumented Lens build of the tested commit:

1. Opening the panel, previewing a range and saving settings made no model calls. The preview reported 20 of 32 eligible runs. Saving again from a stale form returned 409
2. Discovery with Haiku and Qwen drafted 3 tasks in about 30 seconds, with 4 Haiku calls and 1 embeddings call
3. Editing a task and replaying that edit with the old revision returned 409
4. Activating with the backfill box checked classified the 26 runs of the discovery range with 26 Jev calls within 40 seconds
5. Jev was called 26 times in the whole run, all for that backfill, so turning discovery off and then back on without **Resume classification for new runs** made no Jev calls. Resume reopened classification for new runs, and a resume sent with a stale revision was refused
6. Reset and a final turn-off behaved as documented
7. Overview and matrix returned 200. Invalid requests returned 400, and unauthenticated requests 401

## Automated checks at the tested commit

```bash
cd src/worker
cargo fmt --check
cargo clippy --locked --workspace --all-targets -- -D warnings
CLICKHOUSE_STATE_TEST_URL=http://<clickhouse-with-keeper>:8123 \
  cargo test --locked -p lens-server -p litellm-lens -p litellm-traces-clickhouse \
  -p litellm-storage-clickhouse -p lens-profiles -p litellm-traces
cargo run --locked --quiet -p litellm-traces --features schema --bin export-traces-schema -- --ui \
  | diff -u ../../schema/lens-traces.json -
cd ../.. && npm run check:worker-contract && npm run typecheck:ui
npm exec --workspace @litellm/lens-ui -- vitest run src/components/lens/profiles \
  src/components/lens/data/demo src/components/lens/LensWorkspace.integration.test.tsx
```

All of these passed with rustc 1.99.1 on Wolfi. The trace schema export matched with no diff. The ClickHouse suites ran against a Keeper-backed ClickHouse 26.9.6.6, as the CI workflow sets up with `CLICKHOUSE_STATE_TEST_URL`

## Coverage

Coverage counts only production lines this branch changed against `main`, with test modules and test files excluded. Rust is measured with LLVM line coverage

| Scope | Lines | Branches |
| --- | --- | --- |
| Rust, tests only | 2936 / 3399 (86.4%) | not measured on stable Rust |
| Rust, tests plus the live run above | 3278 / 3399 (96.4%) | not measured |
| UI | 553 / 580 (95.3%) | 578 / 669 (86.4%) |

The live run reaches the worker's discovery and classification loops, which have no separate unit harness. Their guards are tested directly

## Limitations

Rust branch coverage is not measured. The Docker `smoke` target and the full workspace test suite are left to CI. The repository has no Codecov configuration, so no coverage gate runs on the pull request. Grouping quality has not been measured against human labels
