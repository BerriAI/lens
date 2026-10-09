# Rust datasets

The Rust application now mounts the seven existing dataset routes when `LENS_ADMIN_TOKEN` is configured. The port began from Lens main `bb5449e8cff067063ea27c87fd2ff0ff3ff8d818` and was reconciled with main `3ca154b6`, including the latest eval scoring, eval trace filtering and UI navigation changes. It preserves the extracted dataset API, case hashes, revision storage keys, defaults, permissions and JSONL exports

Wire records live in `crates/contract/src/datasets.rs`. The `lens-datasets` crate owns case construction, deduplication, scope rules and revision limits. ClickHouse adapters own immutable revisions, latest summaries and finding evidence. The trace adapter reads the existing Rust trace cache in process, follows every cursor page and passes typed spans into the dataset domain. The server shares the existing session, bearer and gateway-delegation authentication implementation

Read and build access still requires a proxy administrator or view-only administrator. Creation and revision saves require a proxy administrator. Existing excluded cases count toward the dataset limit. Saving recalculates content IDs and keeps the first duplicate. Building a whole trace selects the latest LLM span with message-form input, including spans beyond the first 500 records. Finding evidence retains its original provenance and trace reference

The original 25 dataset HTTP recordings are replayed without changing their expected responses. Additional checks cover Unicode sizes and hashes, JSONL field defaults and duplicate fields, legacy execution IDs, body validation, browser origin checks, competing revision writes, independent legacy stored documents and restart persistence. Cross-review found and corrected duplicate JSON field handling and permissive legacy base64 padding

The [application integration log](evidence/rust-datasets-integration.log) records real OTLP ingestion followed by dataset builds from a complete trace, a 502-span trace, an explicit span and finding evidence. It also checks saved datasets and browser sessions after restarting the application router with the same ClickHouse state

The [binary smoke log](evidence/rust-datasets-binary.log) records requests to the actual Rust executable: ingest a trace, build its case, create a dataset, save a revision, export JSONL, retrieve evaluation cases and read the revision after stopping and restarting the process. The test used an isolated ClickHouse database and synthetic test credentials, then removed that database. It did not call a model provider

The final frozen candidate passed 2,344 workspace tests, with zero failures and four existing ignored tests, plus workspace Clippy and formatting. See the [workspace output](evidence/rust-datasets-workspace.log) and [Clippy output](evidence/rust-datasets-clippy.log). The new dataset domain caught all 108 viable mutations; its other 23 mutations did not compile. Dataset storage caught all 21 viable mutations, with five unviable. The [mutation record](evidence/rust-datasets-mutations.json) distinguishes those completed runs from the incomplete historical HTTP run

The server parses wide JSON integers locally with pinned jiter, preserving recorded revision and validation behavior without enabling serde_json arbitrary precision across the workspace. That global feature had broken OTLP parsing during development; the corrected candidate passes all 129 trace capture cases. This checkpoint also enforces a minimum configured delegation-secret length and updates rustls to 0.23.45. The inherited serde_with advisory and remaining release qualification stay open

Reproduce the integrated storage flow from `src/worker` with a ClickHouse/Keeper test service. Omitting `CLICKHOUSE_STATE_TEST_URL` makes the integration fixtures start their own pinned test container

```sh
CLICKHOUSE_STATE_TEST_URL=http://127.0.0.1:18124 \
  cargo test -p litellm-lens --test datasets -- --test-threads=2
CLICKHOUSE_STATE_TEST_URL=http://127.0.0.1:18124 \
  cargo test -p lens-server --test datasets -- --test-threads=2
CLICKHOUSE_STATE_TEST_URL=http://127.0.0.1:18124 \
  cargo test -p litellm-storage-clickhouse --test datasets -- --test-threads=2
cargo test -p lens-datasets
cargo test -p lens-contract --test datasets
```

The Python application and its route inventory remain until the complete Rust application can replace them without breaking the working standalone deployment. The Rust binary still requires the existing worker/control-plane configuration; the binary smoke deliberately left that control plane unavailable and exercised the independently mounted dataset API. SDK dataset resolution, the other Rust APIs and background jobs, production packaging, complete browser/provider tests, gateway embedding, project-releaser, ops, migration and recovery remain part of the [completion plan](completion-plan.md). This increment does not close full-product acceptance
