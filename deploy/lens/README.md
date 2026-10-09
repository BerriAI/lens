# Run Lens

Lens records agent traces, investigates failures, and turns useful examples into datasets. The standalone deployment includes the UI, API and background processing in one Lens service, with ClickHouse storing its data and coordination state

The source installation below builds the current checkout. A published standalone image and release bundle are still being qualified; this guide does not imply that an installable release has shipped

Prefer your coding agent to make the changes? [Set it up for me](../../docs/setup-with-agent.md#start-standalone-lens) has a copyable prompt for standalone setup, an existing LiteLLM deployment, and optional external storage. The commands below remain the shortest manual path

## Start locally

Install Git and Docker with Docker Compose v2, then run:

```sh
git clone https://github.com/BerriAI/lens.git
cd lens
./deploy/lens/start
```

The first start builds Lens, generates private credentials in `deploy/lens/.env`, and starts Lens plus ClickHouse. ClickHouse includes the Keeper configuration Lens needs. You do not need a LiteLLM gateway, PostgreSQL, host Python, Node.js, or provider credentials to record and inspect traces

Open [Lens](http://localhost:4318/ui/) and sign in using `LENS_ADMIN_TOKEN` from `deploy/lens/.env`. Read that file locally; do not paste it into shared logs or commit it. The starter binds Lens to localhost and leaves ClickHouse off the host network

Rerunning the start command preserves the environment file, credentials and ClickHouse volume. The initial image build takes longer than subsequent starts

## Record your first trace

Open **Traces** and choose **Set up tracing** if the setup panel is not already open. Then select your framework, and create a tracing key. Copy the generated configuration into your agent. Keep your agent's existing model endpoint and provider credential; the separate Lens tracing key authorizes telemetry uploads

Run your agent, then use **Check for traces** in setup or open **Traces**. Open the matching run and inspect its messages and tool calls. The **Demo data** switch shows examples; it does not verify your agent connection

For an OTLP exporter, send traces to `http://localhost:4318/v1/traces` with `Authorization: Bearer <tracing-key>`. That address works for an agent running on the same host. An agent in another container or on another machine needs a Lens address reachable from there

You can record traces without an analysis model. When you want Lens to investigate them, follow [Configure analysis models](../../docs/analysis.md), then open **Investigations** and create an investigation with an analysis model and monthly spending limit. [Signals](../../docs/signals.md) use a separate evaluation provider to flag matching traces

## Trace content and size

Lens preserves the full input, output, and attribute values supplied by your instrumentation. Trace uploads, trace reads, eval sessions, and datasets have no default byte or item quotas. Eval runs have no fixed case or trial-count ceilings. Signal definitions and feedback comments have no maximum text length, and signal counts and feedback batches have no fixed item ceiling. Large traces are read in pages, and cache eviction does not make a trace unreadable

Optional operator limits remain available through `OTLP_MAX_SPANS`, `OTLP_MAX_ATTRIBUTES`, `OTLP_MAX_EVENTS`, `OTLP_MAX_LINKS`, `OTLP_MAX_DECODE_DEPTH`, `OTLP_MAX_DECODE_NODES`, `OTLP_MAX_DECODED_SPAN_BYTES`, `CLICKHOUSE_TRACE_MAX_INSERT_BYTES`, `LENS_DATASET_MAX_CASES`, and `LENS_DATASET_MAX_CASE_CHARS`. Leave these unset for the default behavior. Existing explicit values continue to apply

Your exporter, reverse proxy, database, and available memory can still constrain the data Lens receives or processes. Model context windows and the [calculation sandbox](../../docs/sandbox.md) also apply when analyzing traces. Content truncated before it reaches Lens, or stored by an older version with truncation, cannot be reconstructed

## Configure a deployment

For Kubernetes, use the [Lens Helm chart](../../helm/lens/README.md). To move an existing gateway-hosted Lens installation and its saved records, follow [Migrate existing Lens data](../../docs/migration.md) before changing which service owns its writers

The bundled deployment uses one ClickHouse server with Keeper. External storage supports that same topology: one stable endpoint reaching one ClickHouse server, shared by any Lens replicas. Lens payload tables are local to that server; Keeper does not replicate them between ClickHouse servers. Load balancing across ClickHouse nodes, distributed tables and automatic failover to a different ClickHouse server are unsupported. Use the [external ClickHouse settings](../../helm/lens/README.md#use-existing-clickhouse) for the required capabilities and permissions

Keep `deploy/lens/.env` private and retain it alongside your persistent data. The generated secrets have different purposes:

| Setting | Used by |
| --- | --- |
| `LENS_ADMIN_TOKEN` | The administrator signing into Lens and authorized API clients |
| `CLICKHOUSE_PASSWORD` | Lens connecting to its bundled ClickHouse server |
| A tracing key generated in Lens | An agent or exporter uploading traces |
| An analysis or evaluation provider key | Lens sending selected trace content to the configured provider |

For a server deployment, put Lens behind your TLS reverse proxy and set `LENS_PUBLIC_URL=https://lens.example.com` in the environment file. Replace that hostname with yours. This value controls the address displayed to agents and enables secure session cookies. Forward the UI and API paths to port 4318; never expose ClickHouse publicly

If your reverse proxy runs outside this host's network namespace, set `LENS_BIND_ADDRESS` to the private interface it can reach. For a different host port, set `LENS_PORT` and update `LENS_PUBLIC_URL` to the corresponding public address. The default bind address is `127.0.0.1`

Apply environment changes without rebuilding the image:

```sh
docker compose -f deploy/lens/compose.yaml up -d --wait
```

Provider environment variables in `deploy/lens/.env` are passed to the Lens service. Supply production credentials using your platform's secret management where available

The container runs without root or Linux capabilities, with a read-only filesystem and bounded temporary space. Preserve those settings. Investigation calculations require a native Linux kernel with Landlock ABI 3 and seccomp support, as provided by supported Docker Linux hosts. See the [calculation sandbox](../../docs/sandbox.md) for its boundaries and verification command

## Check and restart

From the repository root:

```sh
docker compose -f deploy/lens/compose.yaml ps
curl --fail http://localhost:4318/health/ready
docker compose -f deploy/lens/compose.yaml logs --tail=100 lens
docker compose -f deploy/lens/compose.yaml restart lens
```

`ps` should show both services healthy. The readiness request returns HTTP 200. Verify a stored trace or dataset in the UI after restarting; readiness alone is not a data-recovery test

Stop the installation while retaining its data:

```sh
docker compose -f deploy/lens/compose.yaml down
```

Keep the `clickhouse_data` volume and environment file. Adding `--volumes` to `down` deletes the database, including stored traces, investigations, datasets and sessions

Use [Back up and restore Lens](../../docs/backup.md) before changing a persistent deployment. The helper retains ClickHouse data, coordination state, credentials and the installed Lens image, and restores them into a separate deployment for verification

## Resolve startup problems

| Symptom | Check and action |
| --- | --- |
| Docker is unavailable | Start Docker, verify `docker info`, and rerun `./deploy/lens/start` |
| Port 4318 is in use | Set a free `LENS_PORT` and corresponding `LENS_PUBLIC_URL` in `deploy/lens/.env`, then apply the configuration |
| A required secret is missing | Check the existing environment file. Setup deliberately preserves it rather than rotating credentials behind your back |
| ClickHouse is unhealthy | Run `docker compose -f deploy/lens/compose.yaml logs --tail=100 clickhouse`; check disk space and the mounted Keeper configuration |
| Lens cannot reach ClickHouse | Check both services in `ps`, their logs, and the retained database credential |
| External ClickHouse answers queries but Lens exits before becoming ready | Check the server's KeeperMap and `keeper_map_path_prefix` configuration and the Lens database user's create, insert, read and alter permissions. An analytical query alone does not establish that Lens can initialize its state |
| Setup says no analysis provider is configured | Tracing can still work. Follow the [analysis guide](../../docs/analysis.md) to enable investigations |
| Login works locally but not through the public hostname | Check that `LENS_PUBLIC_URL` matches the browser origin and that your proxy forwards HTTPS correctly |

Official paired Lens and LiteLLM releases share a version and are tested together. Lens can still be installed and operated on its own. Follow the [migration guide](../../docs/migration.md) to transfer existing Lens data
