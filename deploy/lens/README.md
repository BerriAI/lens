# Run the agent-first Lens experiment

This branch opens Lens on a chat composer and a feed of tagging activity and reports from your traces. Deep Agents runs ordinary chat sessions. Hermes runs persistent bots with their own conversation, memory, tools and schedules. Traces remain immutable; agents publish shared annotations, reports and attachments alongside them

This is a separate deployment candidate. Use an isolated installation and new state volumes to try it. Released Lens images do not include this experiment. For Kubernetes, use this checkout's [agent workspace chart settings](../../helm/lens/README.md#agent-workspace)

## Start locally

Use Docker with Compose 2.24 or newer. Supply a dedicated LiteLLM workspace key through `LITELLM_PROXY_API_BASE` and `LITELLM_PROXY_API_KEY` in your shell or secret manager. Give that key access to the chosen model and the MCP servers the agents should use. Lens sends all inference through that gateway. Do not put these values in source files or shell commands shared with others

From this branch's checkout:

```sh
./deploy/lens/start --build
```

The command builds both images and starts Lens, the native agent runtime and ClickHouse. Open [Lens](http://localhost:4318/ui/), then paste the generated `LENS_ADMIN_TOKEN` from `deploy/lens/.env` into the sign-in form. The file is private and survives restarts. It contains the local setup credential, storage password and runtime credential; it does not contain your gateway key

Home keeps the chat composer pinned above a feed of tags and reports. Tag counts show current annotations, not newly processed traces; open a tag to see that bot’s matching traces and evidence. **New Session** opens a separate main-agent conversation. **Bots** shows persistent agents and their chats. **Traces** retains the existing trace browser and connection instructions. Follow the tracing instructions there to send a run from your project

Once a trace is visible, ask Lens:

> Review my recent traces for unsuccessful user tasks. Reuse an existing outcome definition if one fits, annotate the runs with evidence, and publish a report

The report should appear on Home. Open it to inspect evidence, use **Copy for agent** to copy its full report and annotation context, or choose **Discuss with Lens** to start a chat that reads the saved report. Charts use Vega-Lite JSON rendered as SVG with Lens typography and colors. Images appear inline and other attachments are downloadable

## Give a bot an ongoing task

Choose the **+** beside **Bots**, give it a name and describe its task. The model picker lists models available to the shared LiteLLM key. Each bot keeps its chosen model, including its heartbeat and all its chats. For example:

> Review new requests for unsuccessful user tasks. Distinguish a recovered tool error from an unsuccessful final outcome. Reuse the existing outcome definition and write a report when there is something useful to flag

The bot starts immediately. Hermes re-enters its permanent **Bot Chat** every ten minutes using the latest instructions and corrections. A heartbeat can use the model even when it produces no report. Bots are prompted to consider the current time, previous work, new evidence and any cadence you request before publishing. Tagging does not require a report; report timing is agent judgment, not a fixed publication schedule. Use **Pause** in the bot’s task header to stop that bot's process and scheduled work; **Resume** restores its native state

Both the main agent and bots can analyze trace query results with Python in the [existing Lens sandbox](../../docs/sandbox.md). They can filter, group, calculate statistics and produce chart data using the standard library. Code gets only the supplied data and disposable scratch space, with no network, credentials, shell or access to Lens storage. Native terminal, unrestricted code, file writes and script scheduling are disabled. Agents publish annotations and report attachments through Lens tools. Connected MCP and Slack tools retain the permissions of the shared workspace key; sandboxing code does not prevent misuse of an allowed tool

Run native agents in the Linux container, including on macOS through Docker. Direct macOS execution is disabled. The sandbox fails closed when its policy or required Linux kernel support is missing

Ask the main agent, “Show me the unsuccessful requests that Request reviewer found.” It looks up that bot's producer ID and queries the shared annotations. Definitions have stable keys, typed values and aliases. Bots can create new definitions, but cannot silently redefine an existing key. Meaning-based reuse still depends on their judgment; conflicting definitions are surfaced rather than silently combined

Main chats belong to their creator. Bot chats and results are shared in the admin workspace. Only Lens admins can start agent work. The dedicated LiteLLM key determines model and MCP access for the whole experiment

## Connect Slack

Skip this section if you only need the Lens UI. Slack uses the AgentChat Rust SDK with Socket Mode, so no public webhook is required

Choose **Connect Slack** at the bottom of the Lens sidebar. **Create Slack app** opens Slack with the required bot scopes, events and Socket Mode already selected. Choose your workspace and install the app. Copy its Bot User OAuth Token from **OAuth & Permissions** into Lens. In Slack's **Basic Information**, generate an App-Level Token with `connections:write` and paste that into the other field

Choose **Connect Slack** in the form. Lens validates both credentials, encrypts them in its persistent state, and connects immediately without a restart or deployment change. Wait for **Slack is connected**. Existing environment-based configuration still works; a connection saved in Lens takes precedence. Keep the runtime secret with your state backups because it protects the saved Slack credentials

Open an existing chat in Lens and choose **Slack**. Generate a link code, then mention the Slack app with the displayed `lens link …` command in the intended thread within ten minutes. This connects that thread to the same Lens conversation and native session. Only the person who linked it can steer the agent through that thread

A bot can list its authorized threads and send updates there when asked. Outgoing messages have delivery IDs and confirmed receipts. If delivery is uncertain, Lens does not resend automatically. An interrupted inbound Slack turn can require continuing from Lens; check the conversation before repeating it

## Configure the deployment

The workspace default model is `openai/gpt-6.2-astra`; set `LENS_AGENT_MODEL` to another model available to the dedicated gateway key. Choose a different model in the Home or New Session composer before sending the first message. The session keeps that selection when reopened; changing the workspace default affects new sessions and bots. Sessions created before model selection was available continue to use the workspace default. Set `LENS_PUBLIC_URL` to the browser origin so agents produce working trace links. For another local port, set both `LENS_PORT` and `LENS_PUBLIC_URL`

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
| Slack is not connected | Open **Connect Slack** in the sidebar, check the app setup and save valid tokens |
| Existing `.env` lacks `LENS_RUNTIME_SECRET` | Use a fresh experiment directory or add a new private random runtime credential while preserving existing credentials |
| A trace link opens the wrong host | Set `LENS_PUBLIC_URL` to the actual browser origin and recreate Lens |

## Validate candidate images

Build the `runtime` and `agents` targets from `deploy/runtime/Dockerfile`. The packaged smoke test checks authentication, trace ingestion, rejected trace deletion and persistence across restart:

```sh
./deploy/lens/smoke.sh lens:agent-first agent-first lens-agents:agent-first
```

This test uses fixture gateway settings and makes no inference calls. End-to-end native model and MCP checks require the dedicated gateway key. Slack transport tests use a local fixture; sending a real Slack message is a separate deployment verification
