From the repository root, run `docker compose -f runtime/crates/parity/compose.yaml up -d` and `uv sync --dev`
Start the harness with ClickHouse at `127.0.0.1:18124` by default: `PARITY_DATABASE_URL=postgresql://parity:parity-password@127.0.0.1:15433/parity uv run python runtime/crates/parity/harness/serve.py`
Record with `cargo run -p lens-parity -- record --base-url http://127.0.0.1:4100`
Replay with `cargo run -p lens-parity -- replay --base-url http://127.0.0.1:4100` after restarting the harness for a fresh ClickHouse database
