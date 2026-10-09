# Eval runs

The Rust server exposes contract v1 eval routes under `/lens/evals/runs` and dataset resolution/cases under `/lens/datasets`. Requests require `X-Lens-Contract: 1` and the existing Lens authentication layer. Each run belongs to the authenticated credential's team. Standalone administrators use the default local scope, which is separate from named gateway teams

Runs, idempotency keys, trial results, baseline indexes, and scoring leases use the ClickHouse state store. The background closer resolves stored traces, waits for a completed root or 120 seconds without new spans, and calls `lens_evals::evaluate`. Missing or late trials become errors. Comparable completed runs on main supply the baseline

`task_completed` and `called_before` need no model credentials. A `judge` scorer uses the models configured in `LENS_ANALYSIS_MODELS`, including direct providers or an optional gateway. Its explicit model alias takes precedence; an empty model uses `LENS_EVAL_JUDGE_MODEL` or the first configured analysis alias. Missing configuration or a failed judge response fails scoring instead of inventing a score

For gateway-routed analysis, set the analysis model's `api_base` to the gateway API base URL and `api_key_env` to its model credential variable. Configure the same base URL in `LENS_GATEWAY_URL` and share `LENS_GATEWAY_SECRET` with the gateway so judge calls carry a short-lived signed internal marker. See [analysis model configuration](../../docs/analysis.md) for the model settings

Judge requests include the rubric, expected outcome, and recorded trial spans. The signed marker is sent only to the explicitly configured gateway origin and path. The gateway still authenticates and bills the ordinary model credential while keeping internal analysis prompts out of recursive analysis and message logs

The HTTP integration test in `crates/lens/tests/evals.rs` runs 36 cases with three trials through real ClickHouse traces and state, the production closer/scorer, and the SDK development server. It compares shared golden summaries and verifies a critical regression. Set `CLICKHOUSE_STATE_TEST_URL` to an isolated ClickHouse instance with Keeper to reuse it for tests; otherwise the fixture starts its own container
