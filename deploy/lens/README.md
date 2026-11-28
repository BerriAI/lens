# Run Lens

Lens records agent traces, investigates failures, and turns useful examples into datasets. The standalone deployment includes the UI, API and background processing in one Lens service, with ClickHouse storing its data and coordination state

The quickstart below is plain Docker Compose and runs the published Lens image. To pin and verify a release, download `lens-install-VERSION.tar.gz` from [GitHub Releases](https://github.com/BerriAI/lens/releases), [verify it](../../docs/releasing.md#verify-a-release), extract it, and run `./start` inside the extracted directory. The bundle pulls the release image by digest

Prefer your coding agent to make the changes? [Set it up for me](../../docs/setup-with-agent.md#start-standalone-lens) has a copyable prompt for standalone setup, an existing LiteLLM deployment, and optional external storage. The commands below remain the shortest manual path

## Start locally

Install Docker with Docker Compose 2.24 or newer and make sure Docker is running. Then run this in an empty directory:

```sh
curl -fsSLO https://raw.githubusercontent.com/BerriAI/lens/main/deploy/lens/compose.yaml
docker compose up --wait
docker compose logs -n1 credentials
```

The last command prints the address to open and the setup token to paste there:

```
credentials-1  | Open http://localhost:4318/ui/ and paste this setup token: <your token>
```

The Compose file is self-contained, so nothing else has to be downloaded and no checkout is needed. It writes its credentials to `.env` beside itself. In a checkout of this repository, `docker compose up --wait` from the repository root starts the same file and writes `deploy/lens/.env`; the paths below assume a checkout

The first start pulls the latest released image, `ghcr.io/berriai/lens`, generates private credentials in `.env`, and starts Lens plus ClickHouse. It takes under a minute and compiles nothing. ClickHouse includes the Keeper configuration Lens needs. You do not need a LiteLLM gateway, PostgreSQL, host Python, Node.js, or provider credentials to record and inspect traces

Open [Lens](http://localhost:4318/ui/), paste the printed token into the **Setup token** field, and select **Sign in**. The same token is saved as `LENS_ADMIN_TOKEN` in `.env`. The token is printed only when `LENS_PUBLIC_URL` is a localhost address and `LENS_SIGN_IN_LINK` is not `false`. Otherwise the line names the address and tells you to read the token from `.env`. To look around without a token, select **Explore demo data** on the sign-in page, which opens Lens with sample data only. In a checkout the file is `deploy/lens/.env`. Do not paste the token into shared logs or commit it

Lens releases after 0.1.0 also print a link that signs you in without copying the token. Find it with `docker compose logs lens | grep "Sign in"`. The link is printed only when `LENS_PUBLIC_URL` is a localhost address, and setting `LENS_SIGN_IN_LINK=false` in `deploy/lens/.env` turns it off The deployment binds Lens to localhost and leaves ClickHouse off the host network

The credentials are generated once, by a short-lived `credentials` container that writes `deploy/lens/.env` and exits. Starting again keeps that file and the ClickHouse volume. `docker compose down` stops Lens and keeps its data; adding `--volumes` deletes the stored traces

`./deploy/lens/start` starts the same Compose project and remains available. It also records the image it started with in `deploy/lens/.env`, and it is how you run the code in your checkout instead of the published image. The build compiles the Rust backend and can take 20 minutes or more:

```sh
./deploy/lens/start --build
```

A fresh installation started with `--build` keeps rebuilding from source on each start. Set `LENS_IMAGE` in `deploy/lens/.env` to run a different image with either command

## Record your first trace

Open **Home**. It offers two ways to connect an agent and opens on the one that matches how Lens runs: **Through the LiteLLM gateway** when a gateway is connected, where the agent sends telemetry to the gateway with the LiteLLM key it already uses, and **Standalone Lens** otherwise, where the agent sends telemetry straight to Lens with a tracing key

On **Standalone Lens**, choose **Claude Code**, **Codex** or **OpenCode** and then **Copy instructions with new key**. Lens creates a tracing key and copies complete setup instructions, including your Lens address and key. Paste them into that coding agent. The instructions turn on the agent's own telemetry and run one real task. The key will also be in that coding agent's chat history. To keep the key out of chat, choose **Set up manually**, create a key, and save it locally as `LITELLM_TRACING_KEY`. You can then return to **Use a coding agent** and choose **Copy without key**. **Set up manually** also shows the variables for tracing your own application

If your project already has a key, choose **Use an existing key**. **Manage tracing keys** lets you select an existing key to watch for its new traces or revoke a key. Existing secret values cannot be retrieved again. Keep your agent's model endpoint and provider credential; tracing keys authorize telemetry uploads and delivery checks

If you're building an agent yourself, choose **Your own agent** on Home. Select an OpenTelemetry (Python or TypeScript), OpenAI Agents SDK, Vercel AI SDK, or LangGraph example. On **Standalone Lens**, create a tracing key or use one you already have. On **Through the LiteLLM gateway**, supply a LiteLLM virtual key and send telemetry to the gateway instead. Copy the environment and install command, save and run the example, then look for `my-agent` in **Agents**. The examples make a model call with a separate OpenAI provider key and send the test prompt and reply to Lens

Run one actual agent task. The coding agent verifies that exact trace arrived and gives you a link. Home watches for new traces sent with your selected key and offers **Open trace** when one arrives. With a shared existing key, check that the displayed agent is the one you just connected. For manual configuration, choose **Set up manually**. The **Demo data** switch shows examples; it does not verify your agent connection

For an OTLP exporter, send traces to `http://localhost:4318/v1/traces` with `Authorization: Bearer <tracing-key>`. That address works for an agent running on the same host. An agent in another container or on another machine needs a Lens address reachable from there. For Codex CLI and IDE environments, follow [Codex telemetry](../../docs/codex-cli.md)

### Claude Code

Claude Code's telemetry is off by default. Lens records Claude Code turns only when you turn it on, so set the variables in [Claude Code telemetry](../../docs/claude-code.md) wherever you run `claude`. The variables are the same for both setups and only the endpoint and the key differ. With standalone Lens, `OTEL_EXPORTER_OTLP_ENDPOINT` is your Lens address, such as `http://localhost:4318`, and `OTEL_EXPORTER_OTLP_HEADERS` carries a Lens tracing key. When Claude Code's model calls go through a LiteLLM gateway connected to Lens, both point at the gateway instead: the endpoint is the gateway address and the header carries the LiteLLM key Claude Code already uses

Use one setup or the other. If Claude Code's model calls go through a gateway, send its telemetry to that gateway and not straight to Lens. These settings store the prompts you type, Claude Code's replies, the files it reads and the output of its commands. Without them Lens shows no turn or tool steps for Claude Code

You can record traces without an analysis model. When you want Lens to investigate them, follow [Configure analysis models](../../docs/analysis.md), then open **Investigations** and create an investigation with an analysis model and monthly spending limit. [Signals](../../docs/signals.md) use a separate evaluation provider to flag matching traces

To compare models for your agent, open **Model optimizations**. The [model optimizations guide](https://docs.litellm.ai/docs/proxy/lens/model-optimizations) explains Live, Shadow, and Offline comparisons, with runnable examples and the results you can expect

## Trace content and size

Lens preserves accepted input, output, and attribute values without truncation. Oversized requests are rejected with HTTP 413. Trace uploads and internal spend batches accept up to 32 MiB per request; gzip uploads must also expand to at most 32 MiB across all members. Other API request bodies, including dataset, eval, signal and feedback requests, accept up to 2 MiB. Reduce the exporter batch size or split an API operation when it exceeds these limits

OTLP decoding defaults to 16,384 spans per upload, 4,096 attributes, events or links per applicable record, 1,000,000 structural nodes and 64 MiB of decoded span allocations. Configure these through `OTLP_MAX_SPANS`, `OTLP_MAX_ATTRIBUTES`, `OTLP_MAX_EVENTS`, `OTLP_MAX_LINKS`, `OTLP_MAX_DECODE_NODES` and `OTLP_MAX_DECODED_SPAN_BYTES`. `OTLP_MAX_DECODE_DEPTH` can lower the nesting limit, but it cannot exceed the absolute ceiling of 64. Existing `CLICKHOUSE_TRACE_MAX_INSERT_BYTES`, `LENS_DATASET_MAX_CASES` and `LENS_DATASET_MAX_CASE_CHARS` settings also remain available

Database reads buffer at most 64 MiB per response. Ad-hoc SQL readers also enforce 50,000 result rows and 64 MiB of result data in ClickHouse. Narrow the time window, select fewer columns or add a smaller `LIMIT` when a query exceeds these limits. Large traces are read in pages; cache eviction does not remove stored traces. Eval runs have no fixed case or trial-count ceiling, so configure spending controls on the model credential used for scoring

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

## Recorded gateway costs

Lens uses the finalized amounts exported by LiteLLM. On the gateway, set `LITELLM_LENS_URL` to the private Lens service URL. Configure the same `LITELLM_LENS_SERVICE_TOKEN` on both services to enable authenticated spend ingestion. Trace credentials and gateway calls must belong to the same authorized user or key and team for their identifiers to match

New gateway records write their recorded cost and call identifiers into compact Lens storage through the existing background export. Lens acknowledges an upload after both request-log and compact writes succeed. Trace browsing reads the compact records without selecting from `spend_logs`; it does not recalculate prices or accept amounts from agent traces. Calls made without a gateway spend record remain unpriced

### Upgrade and recover historical costs

Upgrade every Lens writer and wait for `/health/ready` before backfilling historical costs. Existing matched historical costs become unavailable until their retained request logs are projected into compact storage. Startup does not scan those logs, and trace browsing has no fallback to the raw table

From the matching source checkout, build the operator command:

```sh
cargo build --manifest-path src/worker/Cargo.toml --locked \
  -p litellm-traces-clickhouse --bin backfill_call_costs --release
```

Run it from a host with private ClickHouse access. Set `CLICKHOUSE_URL` through your secret manager to a credential allowed to select `spend_logs` and insert compact Lens cost records. Set `CLICKHOUSE_DATABASE` if it differs from the default `lens`. Pass one exact team ID, inclusive start and exclusive end as UTC Unix milliseconds. An explicit empty team argument, `''`, selects records with no team

```sh
src/worker/target/release/backfill_call_costs \
  'team-id' 1791504000000 1791590400000
```

Each invocation permits at most a 24-hour window, 20 pages of 500 records and five minutes. Each page also has a ten-second query limit, a 4 MiB response limit, a million-row or 64 MiB scan limit, and a 128 MiB query memory limit. A rejected page leaves the last successful cursor usable; choose smaller time windows when scan limits prevent progress. Only call identifiers, ownership, timestamps and recorded amounts are read, with no prompts or response bodies

Save the printed JSON lines. A cursor is printed and flushed only after its page is persisted. If `complete` is false, a timeout occurs, or the command fails, repeat the same team and time window with the last printed cursor as the fourth argument:

```sh
src/worker/target/release/backfill_call_costs \
  'team-id' 1791504000000 1791590400000 '<cursor>'
```

A cursor is bound to its team and window. Replaying a page or the whole window replaces the existing record identity and version without adding its cost twice. Repeat bounded windows for the historical period you need, then verify trace costs before treating coverage as complete. Expired records cannot be recovered. Backfill restores recorded amounts only; spans with incomplete call evidence remain unmatched

Accepted compact records survive service restarts. The gateway exporter retains its bounded memory queue, three delivery attempts and limited shutdown drain. Callbacks lost before successful ingestion are not recovered by this change. Backfill can recover a raw record retained after an incomplete compact write, but cannot recover a record absent from ClickHouse

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
| Docker is unavailable | Start Docker, verify `docker info`, and rerun `docker compose up --wait` |
| Port 4318 is in use | Set a free `LENS_PORT` and corresponding `LENS_PUBLIC_URL` in `deploy/lens/.env`, then apply the configuration |
| A required secret is missing | Check the existing environment file. Setup deliberately preserves it rather than rotating credentials behind your back |
| ClickHouse is unhealthy | Run `docker compose -f deploy/lens/compose.yaml logs --tail=100 clickhouse`; check disk space and the mounted Keeper configuration |
| Lens cannot reach ClickHouse | Check both services in `ps`, their logs, and the retained database credential |
| External ClickHouse answers queries but Lens exits before becoming ready | Check the server's KeeperMap and `keeper_map_path_prefix` configuration and the Lens database user's create, insert, read and alter permissions. An analytical query alone does not establish that Lens can initialize its state |
| Setup says no analysis provider is configured | Tracing can still work. Follow the [analysis guide](../../docs/analysis.md) to enable investigations |
| Login works locally but not through the public hostname | Check that `LENS_PUBLIC_URL` matches the browser origin and that your proxy forwards HTTPS correctly |

Official paired Lens and LiteLLM releases share a version and are tested together. Lens can still be installed and operated on its own. Follow the [migration guide](../../docs/migration.md) to transfer existing Lens data
