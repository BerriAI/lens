# Historical Rust migration plan

This document records the original backend port. The current product boundary and remaining acceptance checks are in the [completion plan](extraction/completion-plan.md), with results in the [implementation ledger](extraction/implementation-state.json). Use [CONTRIBUTING](../CONTRIBUTING.md) for the current development workflow

The Rust server now owns every Lens route and background task. The Python server and its route registry have been removed. The Python eval SDK and confined investigation interpreter remain supported. Rust parity fixtures remain as regression tests, and the PostgreSQL importer remains for existing installations; PostgreSQL is not a Lens runtime dependency. The current completion plan includes LiteLLM embedding and companion repositories

The steps below are retained as historical context and do not override the current completion plan

Original goal: the repo has two top-level code folders, src/worker (all backend, Rust) and src/ui (all frontend, TypeScript), and zero Python files we wrote. The Rust worker binary is the only server: it serves every /lens/*, /auth/* and /v1/* route, ingest, and the investigation worker. Embedded mode inside the LiteLLM gateway was outside this port's initial scope

Target layout:

    src/
      worker/                Rust workspace (today: runtime/)
        Cargo.toml
        Cargo.lock
        crates/              same crates as today plus the ported ones
          contract/          wire types and JSON schema, the single source of truth
          server/            lens-server, every HTTP route
          auth/ datasets/ evals/ investigations/ ...   pure domain crates, no IO
          storage-clickhouse/ traces/ traces-clickhouse/ traces-cache/
          parity/            dev-only, deleted at the end (see step 3)
          lens/              the binary: config and wiring only
      ui/
        lib/                 today: packages/ui (@litellm/lens-ui, still published)
        app/                 today: apps/web (Next.js host)
      sdk/                   today: packages/sdk (eval SDK, Python, talks to Lens over HTTP only)
    deploy/                  compose, helm values, Dockerfile (Rust binary + ClickHouse only)
    helm/
    docs/
    .github/workflows/
    AGENTS.md  README.md  CONTRIBUTING.md  LICENSE
    package.json             npm workspaces: src/ui/lib, src/ui/app

Deleted by the end: src/litellm_lens/, tests/ (Python), the root pyproject.toml and uv.lock, migrations/ (Postgres), scripts/*.py, deploy/lens/configure.py and test_configure.py, runtime/crates/parity/harness/serve.py, and every reference to Postgres.

Kept on purpose: the eval SDK, because users write their evals in Python. It moves to src/sdk/ in step 1, keeps its own pyproject.toml and uv.lock, and never imports backend code. Also kept: the sandboxed Python interpreter that the investigation agent uses to run model-written analysis code (sandbox.rs, deploy/runtime/python_runtime.py, python_policy.c). It's a runtime the product ships, not our code. Move those two files under src/worker/crates/lens/sandbox/ so nothing Python lives at the top level.

Work in this order. Each step is its own PR (or PR series), branched off the latest main as litellm_lens_<step>, with no slashes. Pull main before each one.

Step 1: move only, no behavior change (one PR)
- git mv runtime -> src/worker, packages/ui -> src/ui/lib, apps/web -> src/ui/app, packages/sdk -> src/sdk, and the sandbox runtime files as above.
- Fix every path that breaks: Cargo workspace paths, .cargo/config.toml, npm workspaces, tsconfig paths, Dockerfile, compose, helm, .github/workflows, scripts, every AGENTS.md link, the README.
- Done when: cargo fmt --check, cargo clippy --all-targets -- -D warnings and cargo test pass from src/worker; the UI typecheck, lint, tests and build pass; docker build passes; the git diff shows renames only, apart from the path fixes.
- Other agents have open work under runtime/ and packages/ui. Land this fast, then post one line in the PR saying open branches should rebase (git follows the renames).

Step 2: Rust owns the contract (one PR)
- Wire types currently come from Pydantic via scripts/generate_lens_contract.py into crates/lens/contract.json and build.rs. Make crates/contract the hand-written source of truth (serde + schemars), export schema/lens.v1.json, have the lens crate use it directly, and delete the generator, contract.json and the typify build step.
- The UI generates its TS types from schema/lens.v1.json.
- Done when: the existing worker tests pass unchanged, and a test fails if the checked-in schema drifts from the Rust types.

Step 3: port every Python module to Rust, then delete it
For each group, in this order:
  a. auth.py, identity.py, clickhouse_state.py (sessions move from Postgres to the ClickHouse state store)
  b. datasets.py, dataset_endpoints.py, dataset_repository.py
  c. tracing_endpoints.py, trace/*, feedback_*, ingestion.py, billing.py, tracing_runtime.py, tracing/*
  d. endpoints.py, state.py, repository.py, signals.py, signal_repository.py, inference.py, sources.py, reviews.py, models.py, agent_contract.py, release.py, prompts/ (investigations control plane)
  e. database.py, migrate.py, context.py, persistence.py, constants.py, migrations/, and everything left
For each group:
  - Record parity fixtures for its routes first (crates/parity record), against the Python code.
  - Port it: pure domain crate with traits for its IO, ClickHouse code in storage-clickhouse or the traces crates, HTTP in crates/server. Where Python called the worker over /internal/*, call the crate in-process instead.
  - Replay the fixtures against the Rust server until they pass, and add a CI job that runs replay against the Rust server.
  - In the same PR: delete the Python files and their tests, and remove the routes from crates/server/python_routes.txt.
  - Port the Python tests' intent into Rust tests (rstest cases). Don't drop coverage.
Known behavior changes that must be separate commits, each updating the fixtures it changes: team keys can read datasets (resolve and cases) scoped to their own team; decide whether a PROXY_ADMIN_VIEW_ONLY user may call POST /lens/datasets/build (today it gets a 200 because the write check is missing).
- Done when python_routes.txt is empty and deleted.

Step 4: cleanup (one PR)
- Delete the root pyproject.toml and uv.lock (not src/sdk's), tests/, scripts/*.py, deploy/lens/configure.py, the parity crate and its harness, and every Postgres reference in deploy/ and helm/.
- Add a CI check that fails if any *.py file exists outside src/sdk/ and src/worker/crates/lens/sandbox/.
- Done when: a fresh clone builds and runs with just cargo, npm and ClickHouse; deploy/lens/compose.yaml starts the Rust binary + ClickHouse and smoke.sh passes against it; the CI python check is green.

Rules for every PR:
- Follow AGENTS.md and src/worker/AGENTS.md: errors in error.rs via thiserror, rstest cases, inline tests for private items, no unwrap outside tests, #![forbid(unsafe_code)] in new crates.
- Run cargo fmt --check, cargo clippy --all-targets -- -D warnings, cargo test, and cargo mutants on any new crate (90% or higher kill rate) before pushing. Put the gate output and the replay output in the PR body.
- Don't merge a PR until its CI run is green. PRs 10 and 11 merged while CI was still running; don't repeat that.
- Conventional commit titles, follow .github/pull_request_template.md if it exists, no Claude or Devin attribution.
- Don't stack PRs. If one depends on another, wait for it to merge.
- Stop and ask before: changing an HTTP response shape that the parity fixtures don't already allow, dropping a feature, or touching the LiteLLM gateway repo.
- After each step, post a short status: what merged, what's next, and anything blocking.
