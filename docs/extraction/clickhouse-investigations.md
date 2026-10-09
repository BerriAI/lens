# Investigation and access storage

Implementation candidate: `d1f42a4a50e5cfb3ba66fcd2f9fc0e51190e9b11`

Lens investigation records, archived jobs, reusable review checkpoints, workers, tracing credentials and browser sessions now use ClickHouse. The API runtime has no SQL database field, and the PostgreSQL driver is an optional migration dependency. Dataset, signal and trace storage already use ClickHouse

The existing state transitions and public models remain in place. A versioned investigation update, removed-job archives and an optional review checkpoint publish through the same KeeperMap commit. A competing worker or attempt invalidates the complete update. Budget settlement retries version conflicts and preserves the existing reservation identity, so duplicate settlement does not charge twice

Schedule, worker and job metadata live in ClickHouse materialized views. Prepared records only become visible when their revision and digest match the committed head. History queries pin the parent investigation before reading archived records and return at most 50 job documents. Finding counts preserve exact trace identity and distinguish unassessed runs from assessed runs with no findings

The tracing-key catalog has the existing 10,000-key cap. Its bounded snapshot is stored as one ClickHouse payload so concurrent creation, listing and revocation share one consistent version. Worker tokens have a separate unique owner record, published atomically with worker creation. Browser sessions store a hash and expiry, and logout publishes a tombstone

## Qualification

The final combined run passes 195 checks in 60.01 seconds: 139 core behavior tests and 56 ClickHouse or API integration checks

The tests run against the pinned ClickHouse 26.9.6.6 integration image with its configured Keeper. PostgreSQL is absent from these runs. Copied behavior cases cover scope and schedule filtering, equal-time paging, legacy schedule repair, competing claims, review reuse and consolidation, obsolete attempts, archived findings and provenance. Additional checks cover failed publication, history ordering, credential limits, worker scope filtering and browser-session authentication through FastAPI

Fifty reservations, each submitted twice concurrently, charge exactly once. A paused progress publication loses to a reassignment and cannot write its checkpoint. A prepared but losing archive remains invisible, and subsequent history contains the winning run contents

The scheduler test grows from 21 to 221 investigations while keeping one due investigation. Document reads remain two records and 104,394 bytes: one selected document and one fresh read for the conditional update. The worker query reads only eligible documents in pages of at most 50

Eight targeted mutations are rejected by behavior assertions: removing attempt fencing, publishing checkpoints separately, omitting archives, ignoring review content versions, restoring revoked access during heartbeat, exceeding the tracing-key cap, exposing uncommitted schedule rows and accepting a session at its expiry. This is not a codebase-wide mutation score

Evidence: [repository tests](evidence/clickhouse-investigations.log), [affected-case recheck](evidence/clickhouse-http-recheck.log), [targeted mutations](evidence/clickhouse-investigation-mutations.json), [scoped type diagnostics](evidence/clickhouse-investigation-types.json) and [unit collection](evidence/python-unit-collection.log). The seven checked Python modules have no type errors; remaining warnings include inherited third-party typing, unused results and adjacent SQL string literals

The independently merged HTTP fixture runner now also uses ClickHouse sessions. All 43 committed authentication and dataset fixtures from `01bd40dfd310a6279488030cc0e509ce2e3db1a4` replay unchanged against a real Uvicorn server, with PostgreSQL absent. The candidate harness is `caaf53d4bc2c4b77257e9a69be2cb6cfa468eb3a`. This covers bearer/cookie/JWT authentication, session revocation and expiry, dataset revisions, exports and role/scope decisions; analysis is deliberately unavailable in this fixture harness

Run `uv run python src/worker/crates/parity/harness/serve.py`, then `cargo run --manifest-path src/worker/Cargo.toml -p lens-parity -- replay --base-url http://127.0.0.1:4100`. Each harness start uses a fresh ClickHouse database. See [replay output](evidence/clickhouse-http-parity.log) and [server log](evidence/clickhouse-http-parity-server.log)

## Limits still open

One combined run passed 186 checks, then encountered an HTTP disconnect during scheduler setup and eight subsequent fixture setup errors. ClickHouse remained running and healthy, with no restart or OOM event. All nine affected checks passed on a targeted recheck, and 1,500 diagnostic requests through fresh client connections had no failures. A subsequent combined run of the same 195 checks passed without changing the workload or restarting ClickHouse. The failed run is retained as [HTTP interruption evidence](evidence/clickhouse-http-interruption.log). Its cause is unresolved, so this is not a deployment reliability qualification

The complete copied Python suite is not yet runnable: a remaining trace-endpoint test imports the gateway runtime and fails collection on a gateway-only dependency. Other copied endpoint and lifecycle fixtures still expect SQL or gateway globals. Their behavior requirements remain open; the former repository unit cases were moved to real ClickHouse integration tests, and legacy PostgreSQL migration checks were retained separately

Schema-version migrations, PostgreSQL import, safe payload and metadata reclamation, sustained capacity, backup/restore and external or multi-node ClickHouse qualification remain open. The current materialized-view initialization is for fresh state databases; it does not backfill an existing installation. Application startup, analysis configuration, background loops, real provider/browser E2E and all cross-repository integration requirements remain open
