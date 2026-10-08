From the repository root, run `docker compose -f runtime/crates/parity/compose.yaml up -d --wait` and `uv sync --dev`. The harness uses ClickHouse with Keeper and does not require PostgreSQL

Start the harness with ClickHouse at `127.0.0.1:18124` by default: `uv run python runtime/crates/parity/harness/serve.py`. Each start creates a fresh isolated database, which is removed on shutdown

Record with `cargo run --manifest-path runtime/Cargo.toml -p lens-parity -- record --base-url http://127.0.0.1:4100`

Replay with `cargo run --manifest-path runtime/Cargo.toml -p lens-parity -- replay --base-url http://127.0.0.1:4100` after restarting the harness for fresh state. The committed fixtures cover authentication and datasets; they do not qualify every Lens endpoint
