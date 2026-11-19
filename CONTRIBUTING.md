# Developing Lens

Lens owns its API, investigation runtime and shared UI. LiteLLM embeds the same UI package and connects through an authenticated server adapter. You do not need a sibling checkout to work on Lens

## Start a development environment

Install Docker with Compose, Node.js 24.14.1 or newer, and npm 11.10.0 or newer. From a fresh checkout:

```sh
npm ci
npm run dev
```

The first start builds the Lens container, creates private credentials in `deploy/lens/.env`, starts Lens and ClickHouse, and starts the UI development server at [http://127.0.0.1:3100/ui/](http://127.0.0.1:3100/ui/). Sign in using `LENS_ADMIN_TOKEN` from that file. The Next.js server forwards API requests to the local Lens container; UI changes reload automatically

If port 4318 is already in use, run `LENS_PORT=4320 LENS_DEV_API_URL=http://127.0.0.1:4320 npm run dev`. The UI still uses port 3100; stop any previous preview using that port before starting

To use an existing Lens image, set `LENS_IMAGE` to its exact local tag or verified release digest before running `npm run dev`. This avoids building the backend. For a UI-only preview using sample data, run `npm run dev:ui` and open [demo data](http://127.0.0.1:3100/ui/?demo=true)

Stop the UI with Ctrl-C. Lens and ClickHouse retain their data and continue running until you stop them:

```sh
docker compose -f deploy/lens/compose.yaml stop
```

Use `npm run dev` again to resume. For a deployment without development tooling, follow [Run Lens](deploy/lens/README.md). Provider configuration is optional until you need [analysis](docs/analysis.md) or [Signals](docs/signals.md)

## Repository layout

| Path | Responsibility |
| --- | --- |
| `src/worker/` | Rust API, domain logic, ClickHouse repositories, ingestion and investigation execution |
| `src/ui/lib/` | Shared Lens React UI consumed by standalone Lens and LiteLLM |
| `src/ui/app/` | Standalone shell and local UI development server |
| `src/sdk/` | Python eval SDK and GitHub action; communicates with Lens over HTTP |
| `deploy/runtime/` | Complete container image with the static UI and calculation sandbox |
| `deploy/lens/` | Compose installation, startup, backup, restore and deployment checks |
| `helm/lens/` | Independently deployable Lens chart |
| `migrations/legacy/` | Historical PostgreSQL source schemas for the one-time import |

All backend code is Rust. The Python eval SDK and confined analysis interpreter under `src/worker/crates/lens/sandbox/` are the only Python exceptions. PostgreSQL is an import source for `lens-migrate`; the Lens server uses ClickHouse only

## Work on the backend

Install Rust 1.99.0, the toolchain used by CI. Run checks from the repository root:

```sh
cargo fmt --manifest-path src/worker/Cargo.toml --all --check
cargo clippy --locked --manifest-path src/worker/Cargo.toml --workspace --all-targets -- -D warnings
cargo test --locked --manifest-path src/worker/Cargo.toml --workspace
```

Storage tests create isolated databases in a Dockerized ClickHouse with Keeper and remove them afterward. Keep Docker running. Set `CLICKHOUSE_STATE_TEST_URL` only when reusing a dedicated test server

To try backend changes with the complete UI and native Linux sandbox, rebuild and restart the service:

```sh
docker build --build-arg LENS_VERSION=local-test -f deploy/runtime/Dockerfile -t lens:local-test .
LENS_IMAGE=lens:local-test npm run dev
```

The source binary can also run on macOS, but its confined Python calculation tool requires a supported native Linux kernel. Use the container for end-to-end investigation checks

### Container build parallelism

The first source build compiles the Rust dependencies and can take many minutes. The container chooses Cargo parallelism from the Docker VM's CPU count and memory limit. It uses at most two jobs on VMs with 8 GiB or less, matching the previous cap, and allows roughly one job per 4 GiB on larger VMs, up to the available CPU count. This is a conservative concurrency policy, not a hard memory limit. The server and migration command build together so Cargo can share dependencies with the same feature set

On a Docker VM with more memory, override the job count explicitly when building, then use the resulting image:

```sh
docker build --build-arg LENS_BUILD_JOBS=4 -f deploy/runtime/Dockerfile -t lens:local .
LENS_IMAGE=lens:local ./deploy/lens/start
```

Choose the job count for the CPUs and memory assigned to Docker, not the host's advertised core count. More jobs can reduce compilation time but increase peak memory use. Keep the default on an 8 GB laptop unless you have measured spare Docker VM memory, or set `LENS_BUILD_JOBS=1` to reduce concurrency further. Normal rebuilds use Docker's layer cache; `--no-cache` forces a fresh compile

Sequential cold builder-stage runs on an 8-CPU, 31-GiB Linux VM took 25m30s before this change and 15m08s with automatic selection of seven jobs, including image export. Combining the production binaries while keeping two jobs took 24m11s. These are single-run observations, not Apple Silicon measurements. Small-memory Docker VMs retain the two-job cap and should not expect the same speedup

To check automatic selection and overrides without recompiling Rust, build the builder stage once, then run the resource-limit tests against it:

```sh
docker build --target builder -f deploy/runtime/Dockerfile -t lens:builder .
node --test deploy/runtime/build-jobs.test.mjs
```

## UI and contract checks

```sh
npm run typecheck:ui
npm run test:ui
npm run build:ui
npm run qualify:ui-package
```

For a specific change, pass the affected paths to Vitest rather than running the whole UI tree. Package qualification packs `@litellm/lens-ui`, installs it in an isolated consumer, and builds that consumer. Production builds write static files to `src/ui/app/out`

Rust owns the public contracts. Regenerate them after changing their owning types:

```sh
npm run generate:worker-contract
npm run check:worker-contract
npm run generate:eval-contract
```

The eval generator requires `uv` for the SDK's pinned generation dependencies. Worker protocol 7 lives in `schema/lens-worker.v7.json`; public eval contract 1 lives in `schema/lens.v1.json`. They are separate interfaces

## Qualify a container

```sh
deploy/lens/smoke.sh lens:local-test local-test
node deploy/lens/recovery-smoke.mjs lens:local-test
docker build --target smoke --build-arg LENS_VERSION=local-test -f deploy/runtime/Dockerfile -t lens:smoke .
docker run --rm --read-only --cap-drop ALL --security-opt no-new-privileges --network none --pids-limit 128 --memory 2g --cpus 2 --tmpfs /tmp:rw,noexec,nosuid,size=1g lens:smoke
```

Run these on the deployment's native Linux architecture. Startup and recovery checks exercise persisted records and credentials. The sandbox check verifies useful calculation and confinement. Run provider, browser, import and connected-mode checks against the release candidate as well

Source qualification does not mean an official release has been published

See [Release Lens](docs/releasing.md) for how a release is published and verified
