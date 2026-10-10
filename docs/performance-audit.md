# Lens performance audit

The observed deployment was database-bound: ClickHouse consumed 0.79–1.00 of its single CPU while the application used 0.02–0.04 CPU. A 24-hour trace-list request failed to finish within 60 seconds despite a dataset of about 6,700 spans. The code repeatedly performs expensive discovery and graph reads, and background workers compete with page loads for the same database

This patch removes avoidable database work and cancels obsolete browser requests. It does not establish a production page-load improvement until the candidate is deployed and measured. The largest remaining change is to make trace-list summaries independently readable without reconstructing every listed graph

The source reference is `9b8bb82ffa8e6659f2664d0184c9a2455a282483`, including merged [PR #42](https://github.com/BerriAI/lens/pull/42). This patch preserves complete traces and histories, adds no data caps, and requires no schema migration or backfill

## Observed production behavior

Measurements were taken on October 9, 2026, initially around 17:07–17:27 Pacific. Both services had one CPU and 2 GiB memory in the same region. These are individual observations, not p95 estimates. Authenticated probes used the deployment administrator scope, so they are not exact replays of the signed-in browser

| Operation | Observed elapsed time | Interpretation |
| --- | ---: | --- |
| Static `/ui/` document | 219 ms | The HTML response was quick; this excludes script execution and data requests |
| `/health/ready` | 179 ms | The service was reachable |
| Authenticated `GET /v1/traces/agents` | 3,529 ms | Agent discovery was slow |
| Authenticated `GET /lens` | 2,995 ms | This probe omitted the UI contract header, so it is not an exact UI replay |
| Authenticated `GET /v1/traces`, 24-hour window | Timeout at 60,001 ms | No usable list response |
| Raw list SQL through the restricted SQL API | 1,895 ms, 50 rows | Does not include graph reconstruction, spend reads, or the full HTTP path |

The first observation window showed ClickHouse memory around 1.22–1.50 GiB and application memory around 250–270 MiB. The database had 13 active source-table parts. Trace sizes were uneven: median 2 spans, p95 about 213, p99 about 2,839, maximum 4,285. A small total row count therefore does not make every graph or captured payload small

Production logs also showed repeated signal/content/readiness failures. Many short `/v1/traces` log entries were POST ingestion requests, so they cannot be used as successful GET-list timings. The restricted SQL API could not expose `system.query_log` or `system.parts`; per-query CPU, bytes read, and execution plans remain unverified in production

The last confirmed live revision during the initial audit was `b5178796b5f01a8ef1eac9df0122743d734d3ab3`. An independent Render deployment of `acb899bbe079a861018e3ba1b3d27f9ac08ebdc7` was subsequently observed building. Record the live revision again before comparing production results

## Changes in this patch

| Change | Why it helps | Correctness boundary |
| --- | --- | --- |
| Count-only automatic readiness | Stops aggregating and transferring full trace attributes merely to check whether ten traces exist | Uses the same sample filters, authorization, time window, and eligibility rules |
| Metadata-free signal discovery | Stops loading unused attributes on recurring signal scans | Preserves identities, ordering, cursors, counts, and root information; classification still fetches content separately |
| Explicit empty-filter fast path | Lets ClickHouse eliminate map reads when no metadata filters are configured | Nonempty filters retain the original predicate |
| Trace IDs in span-batch predicates | Lets the existing TraceId index reject unrelated rows before computing scoped hashes | Keeps team/user authorization and the scoped reference predicate |
| Cursor filtering before sort and deduplication | Avoids reprocessing earlier span IDs for each subsequent page | The cursor fields are the same identity fields used for deduplication |
| Immutable state payload cache | Skips repeated MergeTree payload reads for unchanged KeeperMap revisions | Always reads current heads; caches only digest-verified revisions, not mutable authorization decisions |
| Reuse spans after spend-read failure | Avoids fetching each graph again when a batch cost lookup fails | Retries narrow spend lookups and preserves unknown-cost behavior |
| Browser request cancellation | Stops superseded range, agent, and span requests from continuing in the browser | Preserves previous rows while loading and prevents obsolete results replacing current data |

The state cache is shared by clones of each storage instance, weighted by serialized bytes and key overhead, with 64 MiB capacity and ten-minute idle expiry. Revocation and changed-head tests verify that a warm cache still observes current state. JSON parsing and current-head round trips remain, and cache misses are not coalesced

Browser cancellation does not prove that ClickHouse immediately cancels work already accepted by the server. Trace detail's Suspense requests and several supplementary requests still need cancellation coverage

An experiment with six additional materialized metadata columns was discarded. On ClickHouse 26.9, the original trace-list queries already avoided most unrelated attribute payload bytes in the tested fixtures. That experiment did not justify a migration or an old-data rewrite

## Controlled before and after

These are synthetic local database measurements, separate from production page timings. Both query paths ran on the same seeded database, warmed once, followed by eleven samples per path in alternating order. The environment was an aarch64 Colima VM with two CPUs and 3 GiB memory, ClickHouse 26.9.6.6, a development Rust client, and one client at a time

| Workload | Before median | After median | Measured change |
| --- | ---: | ---: | --- |
| Readiness, 2,000 traces, 64 KiB attributes per trace | 212.30 ms | 15.35 ms | 13.83× faster |
| Readiness, 2,000 traces, 3 KB attributes per trace | 26.70 ms | 14.50 ms | 1.84× faster |
| Signal discovery, 2,000 traces, 64 KiB attributes per trace | 275.00 ms | 22.69 ms | 12.12× faster |
| Selected span batch among 262,144 unrelated rows | 39.12 ms | 9.46 ms | 4.13× faster, single trial |

The larger readiness query reduced ClickHouse bytes read from 131,228,758 to 83,780, about 1,566× fewer. Signal discovery reduced bytes read from 131,228,758 to 112,670, about 1,165× fewer. The selected-span query reduced rows read from 262,146 to 2, about 131,073× fewer. The latter is a scan reduction, not a page-load speedup

A later validation run measured readiness at 249.32 ms versus 20.48 ms, a 12.18× improvement, with the same scan reduction. [Validation and signal-discovery samples](evidence/performance/discovery-validation.json) retain that run

[Readiness samples and environment](evidence/performance/readiness.json) retain the raw timings. The readiness baseline is the full `LensSample` query and the candidate is `LensSampleEligibility`, both on this checkout with the empty-filter optimization. The index baseline used the reference revision's span SQL against the same synthetic fixture. Reverting that predicate made the scan-budget regression test fail

### HTTP service comparison

A separate run exercised two complete local HTTP services sequentially, each with a fresh database and the same synthetic telemetry, using the existing [service benchmark](benchmarks.md). The database container had one CPU and 2 GiB memory inside the two-CPU VM. Lens was a native macOS development binary with two Tokio workers and no host CPU cap. The shared development host was not a dedicated benchmark machine

The fixture contains 100 and then 1,000 traces, each with ten spans. Reads are warmed and repeated eleven times. The list measurement covers every page, not just the first visible page. The runner checks exact trace identities, span counts, complete details, receipts, and exported input/output before reporting latency

[Raw HTTP baseline](evidence/performance/http-baseline-initial.json), [initial candidate](evidence/performance/http-candidate-initial.json), and [test context](evidence/performance/http-context.json) retain the measurements and binary hashes

The initial candidate run showed essentially unchanged warm performance: listing all 1,000 traces took 396 ms before and 388 ms after; cached details took about 4 ms in both versions; span content took about 9–10 ms. This control does not reproduce production's large payloads and active analysis contention, and it does not support a 10× page-load claim. It demonstrates that ordinary warm reads can already be fast on a single database CPU

The finished-patch HTTP run measured 543 ms median and 575 ms p90 for all 1,000 traces. Its p90 exceeded the benchmark allowance of 532 ms against the initial baseline. Receipt, detail, and content also slowed in that stage. [Final samples](evidence/performance/http-candidate-final.json) and the [failed comparison](evidence/performance/http-comparison.json) are retained. The identical baseline binary then slowed to 718 ms median and 924 ms p90 in a [reverse-order recheck](evidence/performance/http-baseline-recheck.json). The [recheck comparison](evidence/performance/http-comparison-recheck.json) passes all ten budgets, but the large baseline variation prevents a reliable end-to-end speedup or regression attribution. The host had about 23 GiB of 24 GiB memory in use and substantial compressed memory during that recheck. Repeat the HTTP gate on an idle isolated host before treating it as release latency evidence

## Remaining work, in priority order

| Priority | Finding and source | Recommended change and acceptance evidence |
| --- | --- | --- |
| P0 | [List orchestration](../src/worker/crates/traces-cache/src/reader.rs) and [summary resolution](../src/worker/crates/traces-cache/src/list.rs) reconstruct complete graphs in sequential batches of 16 | Persist resolved summaries or maintain an independently readable summary projection. Preserve wrapper resolution, call deduplication, unknown spend, ownership, and late-arriving spans. First-page work should scale with returned rows, not their total spans |
| P0 | [Automatic workers](../src/worker/crates/lens/src/local.rs) run three slots; [signal sweeps](../src/worker/crates/lens/src/signals.rs) run concurrently and retry failures at their normal intervals | Coalesce readiness checks across slots, reserve a foreground database budget, and apply capped backoff with jitter to failing sweeps. Test that ingestion and interactive reads progress during sustained background failures |
| P0 | [Detail paging](../src/worker/crates/traces-cache/src/reader.rs) builds the entire resolved graph before returning the first 200 spans | Persist/version graph results or separate the first render from full graph computation. Transport pagination alone does not bound first-page work. Validate deep, wide, resumed, and partially exported traces without dropping spans |
| P1 | [Conversation content](../src/ui/lib/src/components/lens/traces/detail/conversation/useConversationDetails.ts) launches a page of individual span reads; each repeats authorization and database work | Add a scoped batch content API or bounded viewport loading. Preserve order and show individually loaded steps so one slow early item does not hide later results. Measure request count and time to first readable message |
| P1 | [Infinite list refresh](../src/ui/lib/src/components/lens/traces/list/useAgentTraces.ts) re-fetches accumulated pages on live polling | Separate the live head from historical pages, with explicit cursor reconciliation and deduplication. After loading twenty pages, one live tick should have constant cost and keep history available |
| P1 | [Signal batches](../src/ui/lib/src/components/lens/traces/list/useTraceSignals.ts), [feedback](../src/ui/lib/src/components/lens/traces/list/useTraceFeedback.ts), and [findings](../src/ui/lib/src/components/lens/traces/list/useTraceFindings.ts) key requests by the loaded trace set | Keep stable pages/batches and poll visible rows. Avoid rereading old IDs when the final partial batch grows, and avoid scanning all cached signal batches on every render |
| P1 | [Trace availability](../src/ui/lib/src/components/lens/traces/api.ts) fetches a full list from the epoch to answer a boolean | Add a scoped existence endpoint that does not resolve summaries. Ship the new public contract in its own behavior change |
| P1 | [List SQL](../src/worker/crates/traces-clickhouse/query/list_traces.sql) groups aggregate history and references a CTE more than once | Capture production EXPLAIN/query-log evidence, then maintain first-start/identity summary state or a suitable projection. Do not blindly push the lower time bound before `min(StartTs)`: that changes which resumed traces belong in the window |
| P1 | [Graph traversal](../src/worker/crates/traces/src/resolve/graph.rs) and [resolution](../src/worker/crates/traces/src/resolve/resolution.rs) repeatedly materialize ancestor and descendant vectors | Benchmark deep chains and many wrappers, then use short-circuit traversal and memoized ownership where semantics permit. Preserve missing-parent, cycle, wrapper, and spend-attribution behavior. Superlinear behavior is a source-level risk, not a measured production attribution |
| P1 | Mutable application records use [KeeperMap heads and immutable JSON blobs](../src/worker/crates/storage-clickhouse/src/state.rs) | Measure head reads, JSON parsing, whole-history decoding, and write fsync independently. Introduce scoped list projections and paged histories instead of loading complete records to list investigations. Consider a transactional control store if those operations dominate after caching |
| P2 | Agent/status/search filtering is applied to already loaded traces | Add server-side scoped filters with stable pagination and indexed predicates; measure list cardinality and filtering cost |
| P2 | Workspace tabs are imported eagerly; asset caching/compression can improve startup | Measure actual transferred chunks and browser execution before splitting bundles. Export-directory size is not initial network payload size |

A cold 50-row list can require one list query, four sequential span-batch reads, and four spend lookups, plus pagination or fallback work. Summary caches improve repeated reads but do not bound the cold path

The current eight read slots and ten-second queue allowance can amplify latency under saturation. Increasing query concurrency on one fully occupied CPU is unlikely to fix it. Measure queue time separately from execution and prioritize foreground work before raising concurrency

A modest database CPU increase may reduce contention immediately, but it needs a measured rollout and cost decision. This audit does not establish that larger hardware is required for a dataset this small

## Performance budgets and measurement plan

Proposed acceptance targets are visible click feedback within 100 ms, first 50 traces at p95 below 500 ms, warm graph detail below 200 ms, and first selected content below 500 ms. These are targets, not achieved production guarantees

Measure the browser interaction and its dependent endpoints together: open Traces, change the time range, switch agents, open a large trace, open Conversation, select a span, load more, and leave Live running after several historical pages. Also measure Agents, Investigations, Datasets, Evals, and Settings while traces ingest and background work is active

Use the same account, scope, range, dataset, hardware, browser, and cache conditions before and after. Record first useful paint as well as request duration, cancellation, errors, and stale-result behavior. Include cold and warm runs, raw samples, median and p95, exact revisions, and at least thirty repetitions for a production p95 estimate. Record failures rather than dropping them from the distribution

For server attribution, add or capture a request correlation ID, read-queue wait, current-head lookup, payload lookup, graph resolution, database elapsed time, rows/bytes read, response bytes, and cache hit/miss. Do not log captured input/output, credential values, or full SQL with customer identifiers

With a direct administrator ClickHouse connection, these read-only diagnostics identify expensive query families without exporting trace content:

```sql
SELECT normalized_query_hash,
       count() AS queries,
       quantiles(0.5, 0.95)(query_duration_ms) AS duration_ms,
       sum(read_rows) AS rows_read,
       sum(read_bytes) AS bytes_read,
       max(memory_usage) AS peak_bytes
FROM system.query_log
WHERE event_time >= now() - INTERVAL 15 MINUTE
  AND type = 'QueryFinish'
  AND is_initial_query = 1
GROUP BY normalized_query_hash
ORDER BY bytes_read DESC
LIMIT 20;

SELECT table, count() AS active_parts, sum(rows) AS rows,
       sum(bytes_on_disk) AS bytes_on_disk
FROM system.parts
WHERE active AND database = currentDatabase()
GROUP BY table;
```

Use the database's configured logging interval and inspect failed queries separately. The public trace SQL endpoint does not expose these system tables. Run `EXPLAIN indexes = 1` on the exact parameterized list and span queries through the administrator connection, with private parameters kept out of shared artifacts

## Reproduce and roll out

From the repository root, with Docker available, run the targeted database regressions:

```sh
cargo test --manifest-path src/worker/Cargo.toml --locked \
  -p litellm-traces-clickhouse --test load -- --test-threads=1 --nocapture
cargo test --manifest-path src/worker/Cargo.toml --locked \
  -p litellm-storage-clickhouse --test state_transport
cargo test --manifest-path src/worker/Cargo.toml --locked \
  -p litellm-traces-cache --test read
```

The load tests assert data correctness and scan budgets instead of fragile wall-clock thresholds. They also emit raw timings. Follow the [paired HTTP procedure](benchmarks.md) for complete-service comparisons on isolated databases

Before deployment, record the active application revision and resource settings, retain a rollback image, and check the patch's CI and review results. Deploy the application and UI together. No database rewrite is required. Verify readiness, new ingestion, list counts, complete large-trace pagination, selected content, and immediate access revocation

Repeat the production measurement plan with background workers active. Roll back the application if error rates, data completeness, authorization behavior, or latency regress. A successful local benchmark alone is insufficient evidence that the production traces page is fixed

## Validation status

Local validation passed 51 ClickHouse query/load/read tests, 38 state-transport tests, 31 trace-cache tests, two signal runtime cases, and 81 targeted UI integration cases. UI type checking, the production UI build, generated contract checks, Rust formatting, and Clippy for all targets in the five affected packages passed

The signal runtime cases include a 64 KiB unused attribute and verify that classification still receives the evidence content, stores results, and handles provider failure. Trace read coverage includes large traces, pagination during ingestion, tenant collisions, and spend ownership. The full workspace Clippy attempt hit local disk exhaustion; the affected-package Clippy run passed after disabling incremental output. CI still owns the full workspace and container qualification

The audit Markdown was rendered and inspected at desktop and narrow widths, and repository-relative links were checked. Browser interaction tests against the changed production deployment and production after measurements remain pending
