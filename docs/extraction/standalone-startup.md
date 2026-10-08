# Standalone development startup

The Python application now serves the shared UI, authentication, investigation, dataset, feedback and trace routes. Its lifecycle owns the ClickHouse state client, the Rust control client and both signal tasks. Runtime identity and ingestion-key publication use this configured connection. Readiness requires ClickHouse state access plus a Rust service with storage, current credentials and the matching release/protocol

This is a development tracing path, not the published standalone product. Analysis-provider configuration and the model/key support APIs are not implemented yet. The default analysis adapter rejects investigation setup. The copied onboarding still contains gateway instructions and an unconfigured Rust worker logs unsuccessful claim attempts. The signal tasks are wired and cancelled on shutdown, but their provider-backed operation has not been qualified here

## Run from this checkout

Install the Python, Node and Rust prerequisites described in the repository, then run these commands from its root

```sh
uv sync --dev
npm ci
npm run build:ui
cargo build --manifest-path runtime/Cargo.toml -p litellm-lens --bin litellm-lens --locked
docker compose -f tests/integration/clickhouse.compose.yaml up -d --wait
mkdir -p .lens-dev
uv run python - <<'PY'
from pathlib import Path
from secrets import token_urlsafe

path = Path('.lens-dev/startup.env')
if not path.exists():
    with path.open('x') as destination:
        path.chmod(0o600)
        destination.write(
            f'export LENS_ADMIN_TOKEN={token_urlsafe(40)}\n'
            f'export LITELLM_LENS_SERVICE_TOKEN={token_urlsafe(40)}\n'
            'export CLICKHOUSE_URL=http://127.0.0.1:18124\n'
            'export CLICKHOUSE_DATABASE=lens_startup_dev\n'
            'export LENS_PUBLIC_URL=http://127.0.0.1:4100\n'
            'export LITELLM_URL=http://127.0.0.1:4100\n'
            'export LITELLM_LENS_URL=http://127.0.0.1:14318\n'
            'export LITELLM_LENS_PUBLIC_URL=http://127.0.0.1:14318\n'
            'export LITELLM_LENS_LISTEN=127.0.0.1:14318\n'
            'export LENS_UI_DIRECTORY=apps/web/out\n'
        )
PY
source .lens-dev/startup.env
export LITELLM_RELEASE_TAG="sha-$(git rev-parse HEAD)"
uv run uvicorn litellm_lens.application:create_app --factory --host 127.0.0.1 --port 4100
```

In a second terminal, from the same checkout

```sh
source .lens-dev/startup.env
export LITELLM_RELEASE_TAG="sha-$(git rev-parse HEAD)"
runtime/target/debug/litellm-lens
```

The retained `LITELLM_URL` variable points to this Python API, not a gateway. Both processes must use the same database, service secret and release identity. Credentials refresh every 30 seconds, so readiness can initially return 503 while startup converges

Open <http://127.0.0.1:4100/ui/> and sign in using `LENS_ADMIN_TOKEN` from the local configuration file. Tracing credentials remain separate: create one with `POST /lens/tracing/keys`, send OTLP to `http://127.0.0.1:14318/v1/traces`, then use the UI or `GET /v1/traces` on port 4100 to read it. API clients authenticate with the setup token as a bearer credential. Browser writes use the session cookie and must originate from `LENS_PUBLIC_URL`

The application creates its database and current fresh schema if absent. It does not rotate configured credentials or delete records on restart. This does not implement versioned schema upgrades, legacy imports or the production migration boundary

## Evidence and limits

`tests/integration/test_application.py` exercises the assembled application against ClickHouse. It covers first sign-in, cookie reuse, key and dataset persistence across API lifespans, explicit runtime configuration, private credential snapshots and readiness rejection for missing storage, stale credentials and incompatible protocol/release. Dataset routes are registered before the general investigation route, preserving `/lens/datasets`

The focused configuration/application/authentication run passed 15 checks. Three targeted mutations were rejected by assertions: shadowing the dataset route, bypassing service authentication and ignoring protocol compatibility. See [test output](evidence/standalone-startup-tests.log) and [mutation results](evidence/standalone-startup-mutations.json)

A live Uvicorn process, freshly built Rust binary and ClickHouse 26.9.6.6 were also exercised without a gateway or PostgreSQL. A copied OpenAI Agents fixture was replayed over OTLP using a newly created tracing key. Python returned the stored run with six spans, and the browser opened its steps. No provider call was made. The browser exposed a standalone sign-in cache bug: clearing the active session query detached its observer, leaving the sign-in screen visible after a successful response. Preserving that query fixes the transition, which was checked again without reloading

The browser also exposed unfinished setup dependencies: `/models` and `/model_group/info` return 404 until the standalone analysis APIs are implemented, and the copied guide still refers to a gateway. A narrow viewport showed overlapping trace-table headings; baseline comparison and resolution remain open. These observations are not full browser parity, provider E2E, signal lifecycle qualification or a completed S1/S2/F1 acceptance case
