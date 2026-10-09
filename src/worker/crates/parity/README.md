# Recorded behavior fixtures

This development-only crate preserves HTTP responses recorded from the original Lens implementation. Rust integration tests replay the fixtures against the current routers with real ClickHouse storage. The former Python application and recording harness have been removed; Git history retains their provenance

Run the replaying integration suites from the repository root with Docker available:

```sh
cargo test --locked --manifest-path src/worker/Cargo.toml -p lens-server -p litellm-lens
```

The fixture groups cover authentication, datasets, investigations, feedback, activity and Signals. They supplement live provider, browser, concurrency and recovery checks. Preserve meaningful behavioral assertions when updating a contract; do not regenerate expected responses to hide a regression
