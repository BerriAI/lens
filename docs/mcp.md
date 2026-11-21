# Lens MCP server

Lens serves a read only [MCP](https://modelcontextprotocol.io) endpoint so a coding agent can look up its own runs: list traces, open one with its steps and cost, and read its turns. It is part of the Lens server, so there is nothing to install and it always matches your Lens version

## Add it to Claude Code

Use the Lens address and `LENS_ADMIN_TOKEN` from `deploy/lens/.env`:

```sh
claude mcp add --transport http lens http://localhost:4318/lens/mcp \
  --header "Authorization: Bearer <LENS_ADMIN_TOKEN>"
```

Run `claude mcp list` and check that `lens` shows as connected. Then ask something that needs trace data, for example "Use Lens to find my last failed run and tell me which tool call failed and what the run cost"

Any MCP client that speaks Streamable HTTP works the same way: send JSON-RPC with `POST` to `/lens/mcp` and the same `Authorization` header

## Credentials

The endpoint takes the same credentials as the Lens trace API. `LENS_ADMIN_TOKEN` reads every trace, and whoever runs the Lens deployment holds it. A tracing key from the Tracing keys page can only send traces, so it is rejected here with `401`. Lens also accepts the signed identity a connected LiteLLM gateway sends for a user, and then returns only the traces that user and their teams may see

## Tools

| Tool | What it returns |
| --- | --- |
| `list_agents` | The agents that sent traces, with run and failure counts |
| `list_traces` | Recent traces, newest first, with input preview, status, models, token counts and cost. Filter with `agent`, widen the window with `hours` (24 by default) and page with `cursor` |
| `get_trace` | One trace: its summary with total cost, its agents and its spans in order. Page with `cursor` and `page_size` |
| `get_trace_conversation` | The turns of one trace: each user input with the agent's answer |
| `get_span` | One span with its full input, output, attributes and cost |

Cost is the `spend` field in USD. It is filled when the calls went through a LiteLLM gateway, which is where the price is known

Every tool is read only. The endpoint answers each request with one JSON response, keeps no session between requests and does not open an event stream, so `GET /lens/mcp` returns `405`
