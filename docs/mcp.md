# Lens MCP server

Lens serves a read only [MCP](https://modelcontextprotocol.io) endpoint so a coding agent can look up its own runs: list traces, open one with its steps and cost, and read its turns. It is part of the Lens server, so there is nothing to install and it always matches your Lens version

## Connect your agent

Open **Connectors**, directly above **Settings**, then choose **Lens MCP**. Setup detects whether you are using the gateway or standalone Lens

1. Choose your agent app, name the connection, and select an expiry
2. Create the connection. Standalone Lens shows a dedicated read-only token once; save it in your secret store. Through the gateway, use a LiteLLM key belonging to your signed-in user
3. Set the environment variable shown in Setup, then copy the generated client configuration. Commands and agent prompts contain no live credentials
4. Copy the starter prompt into your agent. Setup shows **Connected** only after Lens records a successful tool call in the selected activity window

For example, a standalone Codex connection uses its named endpoint:

```sh
export LENS_MCP_TOKEN='<your saved MCP token>'
codex mcp add lens --url 'https://lens.example.com/lens/mcp/<connection-id>' \
  --bearer-token-env-var LENS_MCP_TOKEN
```

The environment variable must be available when the agent starts. Claude Code expands and stores the authorization header when its install command runs; update that configuration when replacing the credential. Cursor uses an environment reference in its MCP configuration

Setup also includes reusable instructions for `AGENTS.md` or `CLAUDE.md`. A successful empty result means the connection works but there are no matching traces. Send a run from your project or check the agent and time range

## Access and usage

**Access** lists connection owners, scopes, expiry, and revocation. Gateway connections check the current caller identity and team permissions on every request. A different user's key cannot use your named connection. Standalone tokens can read all traces but are accepted only by MCP, not by settings or ingestion APIs. Standalone owner labels and client names are labels, not verified identities

Revocation immediately blocks the named connection. Revoking a gateway connection does not revoke the underlying LiteLLM key, its other permissions, or its access through the original `/lens/mcp` endpoint. Manage that key in LiteLLM when you need to remove its gateway access entirely

**Usage** shows tool calls, errors, and last use within an explicit time window, with a paginated request log. Activity is retained for 30 days. It stores metadata only, never tool arguments, results, prompts, authorization headers, or raw tokens. Denied attempts are associated with the targeted connection; they do not establish that its owner made the request

Administrators can view all connections and activity. Members see their own. Viewer roles cannot create or revoke connections. Existing credentials appear after their next request; shared credentials cannot identify individual people

## Existing clients

The original `/lens/mcp` endpoint continues to accept the deployment admin token and signed gateway identities. Dedicated standalone MCP tokens also work there and retain their expiry and revocation checks. Tracing keys are write-only and cannot read through MCP

## Tools

| Tool | What it returns |
| --- | --- |
| `list_agents` | The agents that sent traces, with run and failure counts |
| `list_traces` | Recent traces, newest first, with input preview, status, models, token counts and cost. Filter with `agent`, widen the window with `hours` (24 by default) and page with `cursor` |
| `get_trace` | One trace: its summary with total cost, its agents and its spans in order. Page with `cursor` and `page_size` |
| `get_trace_conversation` | The earlier turns of the session a trace belongs to: each previous user input with the agent's answer, oldest first. The trace's own input and output are in `get_trace` and `get_span` |
| `get_span` | One span with its full input, output and attributes. Its cost is on the span in `get_trace` |

Cost is the `spend` field in USD. It is filled when the calls went through a LiteLLM gateway, which is where the price is known

Every tool is read only. The endpoint answers each request, or each batch of up to 16 requests, with one JSON response, keeps no session between requests and does not open an event stream, so `GET /lens/mcp` returns `405`
