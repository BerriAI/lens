# Eval runs

The Rust server exposes contract v1 eval routes under `/lens/evals/runs` and dataset resolution/cases under `/lens/datasets`. Requests require `X-Lens-Contract: 1` and the existing Lens authentication layer. Each run belongs to the authenticated credential's team

Runs, idempotency keys, trial results, baseline indexes, and scoring leases use the ClickHouse state store. The background closer resolves stored traces, waits for a completed root or 120 seconds without new spans, and calls `lens_evals::evaluate`. Missing or late trials become errors. Comparable completed runs on main supply the baseline

`task_completed` and `called_before` need no model credentials. A `judge` scorer calls `LITELLM_URL/chat/completions` using the operator's `LITELLM_API_KEY`. Its explicit model takes precedence; an empty model uses `LENS_EVAL_JUDGE_MODEL`. Missing configuration or a failed judge response fails scoring instead of inventing a score

Judge requests include the rubric, expected outcome, and recorded trial spans. They are marked as internal eval traffic and carry Lens team/run/case metadata. That attribution does not change the gateway credential's billing or access permissions

The HTTP integration test in `crates/lens/tests/evals.rs` runs 36 cases with three trials through real ClickHouse traces and state, the production closer/scorer, and the SDK development server. It compares shared golden summaries and verifies a critical regression. Set `CLICKHOUSE_STATE_TEST_URL` to an isolated ClickHouse instance with Keeper to reuse it for tests; otherwise the fixture starts its own container
