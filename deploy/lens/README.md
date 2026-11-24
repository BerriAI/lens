# Run the agent-first Lens experiment

This branch opens Lens on a chat composer and a feed of reports from your traces. Deep Agents runs ordinary chat sessions. Hermes runs persistent bots with their own conversation, memory, tools and schedules. Traces remain immutable; agents publish shared annotations, reports and attachments alongside them

This is a separate deployment candidate. Use an isolated installation and new state volumes to try it. Released Lens images do not include this experiment. For Kubernetes, use this checkout's [agent workspace chart settings](../../helm/lens/README.md#agent-workspace)

## Start locally

Use Docker with Compose 2.24 or newer. Supply a dedicated LiteLLM workspace key through `LITELLM_PROXY_API_BASE` and `LITELLM_PROXY_API_KEY` in your shell or secret manager. Give that key access to the chosen model and the MCP servers the agents should use. Lens sends all inference through that gateway. Do not put these values in source files or shell commands shared with others

From this branch's checkout:

```sh
./deploy/lens/start --build
```

The command builds both images and starts Lens, the native agent runtime and ClickHouse. Open [Lens](http://localhost:4318/ui/), then paste the generated `LENS_ADMIN_TOKEN` from `deploy/lens/.env` into the sign-in form. The file is private and survives restarts. It contains the local setup credential, storage password and runtime credential; it does not contain your gateway key

Home has the chat composer and reports. **New Session** opens a separate main-agent conversation. **Bots** shows persistent agents and their chats. **Traces** retains the existing trace browser and connection instructions. Follow the tracing instructions there to send a run from your project

Once a trace is visible, ask Lens:

> Review my recent traces for unsuccessful user tasks. Reuse an existing outcome definition if one fits, annotate the runs with evidence, and publish a report

The report should appear on Home. Open it to inspect evidence, use **Copy for agent** to copy its full report and annotation context, or choose **Discuss with Lens** to start a chat that reads the saved report. Images appear inline and other attachments are downloadable

## Give a bot an ongoing task

In **Bots**, choose **Create bot**, give it a name and describe its task. For example:

> Review new requests for unsuccessful user tasks. Distinguish a recovered tool error from an unsuccessful final outcome. Reuse the existing outcome definition and write a report when there is something useful to flag

The bot starts immediately. Hermes re-enters its permanent **Bot Chat** every ten minutes using the latest instructions and corrections. It can create its own native scheduled jobs. A heartbeat can use the model even when it produces no report. Use **Pause** to stop that bot's process and scheduled work; **Resume** restores its native state

Ask the main agent, “Show me the unsuccessful requests that Request reviewer found.” It looks up that bot's producer ID and queries the shared annotations. Definitions have stable keys, typed values and aliases. Bots can create new definitions, but cannot silently redefine an existing key. Meaning-based reuse still depends on their judgment; conflicting definitions are surfaced rather than silently combined

Main chats belong to their creator. Bot chats and results are shared in the admin workspace. Only Lens admins can start agent work. The dedicated LiteLLM key determines model and MCP access for the whole experiment

## Connect Slack

Skip this section if you only need the Lens UI. Slack uses the AgentChat Rust SDK with Socket Mode, so no public webhook is required

Create or select a Slack app with Socket Mode enabled. Give its app-level token `connections:write`. Give the bot `app_mentions:read`, `chat:write`, and the history scopes for the conversation types you intend to use (`channels:history`, `groups:history`, `im:history`, or `mpim:history`). Subscribe to `app_mention` and the corresponding message events, install the app, and invite it to the intended channel

Load the tokens through `LENS_AGENT_SLACK_BOT_TOKEN` and `LENS_AGENT_SLACK_APP_TOKEN`, then apply the configuration:

```sh
docker compose -f deploy/lens/compose.yaml up --detach --wait
```

Open an existing chat in Lens and choose **Slack**. Generate a link code, then mention the Slack app with the displayed `lens link …` command in the intended thread within ten minutes. This connects that thread to the same Lens conversation and native session. Only the person who linked it can steer the agent through that thread

A bot can list its authorized threads and send updates there when asked. Outgoing messages have delivery IDs and confirmed receipts. If delivery is uncertain, Lens does not resend automatically. An interrupted inbound Slack turn can require continuing from Lens; check the conversation before repeating it

## Configure the deployment

The default model is `openai/gpt-6.2-astra`; set `LENS_AGENT_MODEL` to another model available to the dedicated gateway key. Set `LENS_PUBLIC_URL` to the browser origin so agents produce working trace links. For another local port, set both `LENS_PORT` and `LENS_PUBLIC_URL`

To run previously built candidates, export both `LENS_IMAGE` and `LENS_AGENTS_IMAGE` with references from the same source revision, then run `./deploy/lens/start`. The public release tag is not an agent-first candidate

The core process holds gateway and storage credentials. Native agents run in a separate container under separate Linux users with private working directories. They receive actor-scoped Lens credentials and a Lens model endpoint. They can execute code and use permitted tools; traces can only be read through their Lens tool interface. Use a dedicated gateway key and an isolated deployment while evaluating this shared admin experiment

The Compose limits are 2 CPUs and 2 GiB for Lens, 2 CPUs and 4 GiB for native agents, and 4 GiB for ClickHouse. More simultaneous bots need more memory. Bot working files, native sessions and schedules persist in `agent_data`; shared annotations, reports, artifacts, conversations and Slack mappings persist in `lens_data`; traces and ingestion credentials persist in `clickhouse_data`

## Check and restart

```sh
docker compose -f deploy/lens/compose.yaml ps
curl --fail http://localhost:4318/health/ready
docker compose -f deploy/lens/compose.yaml restart agents lens
```

All three long-running services should be healthy. Reopen a saved chat and a known trace to verify retained data. The core reconciles enabled bots every thirty seconds, including after the native runtime restarts. An in-flight turn interrupted by a restart is marked interrupted, rather than replayed automatically

```sh
docker compose -f deploy/lens/compose.yaml down
```

This stops the experiment and retains its volumes. Keep all three data volumes and `.env`. Adding `--volumes` deletes them. Before replacing persistent storage, stop the services and snapshot all three volumes plus the generated credentials together. The older ClickHouse-only backup helper does not capture native agent state

| Symptom | Action |
| --- | --- |
| Gateway environment is missing | Export the two gateway variables in the shell running Compose |
| Models work but MCP discovery is denied | Grant the dedicated key the intended MCP servers in LiteLLM |
| A bot says the runtime is unavailable | Check `agents` health and logs, then resume the bot |
| Slack is absent from the chat header | Configure both Slack tokens and recreate the Lens service |
| Existing `.env` lacks `LENS_RUNTIME_SECRET` | Use a fresh experiment directory or add a new private random runtime credential while preserving existing credentials |
| A trace link opens the wrong host | Set `LENS_PUBLIC_URL` to the actual browser origin and recreate Lens |

## Validate candidate images

Build the `runtime` and `agents` targets from `deploy/runtime/Dockerfile`. The packaged smoke test checks authentication, trace ingestion, rejected trace deletion and persistence across restart:

```sh
./deploy/lens/smoke.sh lens:agent-first agent-first lens-agents:agent-first
```

This test uses fixture gateway settings and makes no inference calls. End-to-end native model and MCP checks require the dedicated gateway key. Slack transport tests use a local fixture; sending a real Slack message is a separate deployment verification
