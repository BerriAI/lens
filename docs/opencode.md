# OpenCode

How Lens records OpenCode depends on the OpenCode version. Run `opencode --version` first. OpenCode 2.3.2 and later use the Lens plugin. OpenCode 1.x uses OpenCode's own telemetry, described in the second half of this page

Both store what you type, what the model replies, the files OpenCode reads and the output of the commands it runs. That content can include source code or secrets, so use them only with a Lens deployment where you intend to keep it

## OpenCode 2.3.2 and later: the Lens plugin

OpenCode 2 sends no prompts, replies or tool content in its telemetry, so Lens records it with [a plugin](https://github.com/BerriAI/litellm-lens-plugins/tree/main/opencode) instead. Install it, tell it where to send traces, and restart OpenCode's background service from that same terminal so it starts with the settings:

```sh
opencode plugin add github:BerriAI/litellm-lens-plugins
export LITELLM_LENS_OPENCODE_URL="<address>"
export LITELLM_LENS_OPENCODE_API_KEY="<key>"
opencode service restart

opencode run "Run the date command, then tell me what it printed."
```

OpenCode runs sessions in a background service that keeps running after OpenCode closes, and the plugin reads the two variables from that service, not from the terminal you type in. Add both exports to your shell profile, and whenever no trace arrives, run `opencode service restart` from a terminal where both are set. A service started without them records nothing, and the plugin writes the reason to `~/.local/state/litellm-lens/opencode.log`

The plugin stays off until both variables are set. Use the address and key from the [table below](#address-and-key). Leave `experimental.openTelemetry` and the `OTEL_EXPORTER_OTLP_*` variables unset on OpenCode 2, because the plugin replaces them

A session is one trace, and each prompt you send is a turn in it that shows what you asked and the final reply. Inside a turn, each model call is a `chat <model>` step with the system prompt, the messages added since the previous reply, the reply and its tokens, and each tool call is an `execute_tool <name>` step with its input and output. Earlier messages stay on the earlier model calls, so the thread reads top to bottom without repeats. A subagent's steps sit under the tool call that started it, in the same trace. The call OpenCode makes to name a session is listed as a model call without content. Through a LiteLLM gateway, each model call also shows the cost the gateway billed, and the trace belongs to the key, user and team of the LiteLLM key that sent it

Each turn is sent when it ends. To stop recording, unset the two variables or run `opencode plugin remove github:BerriAI/litellm-lens-plugins`

Verified with OpenCode 2.3.2 and plugin 0.1.2 through a LiteLLM gateway

## Address and key

| Setup | `<address>` | `<key>` |
| --- | --- | --- |
| OpenCode routed through a LiteLLM gateway [connected to Lens](https://docs.litellm.ai/docs/proxy/lens/deployment/litellm) | Your gateway address, such as `http://localhost:4000` | The LiteLLM key OpenCode already uses for model calls |
| Standalone Lens | Your Lens address, such as `http://localhost:4318` | A Lens tracing key |

## OpenCode 1.x: telemetry

OpenCode 1.x sends no telemetry by default. Turn on its OpenTelemetry spans in `opencode.json`, in your project or in `~/.config/opencode/opencode.json`, and keep any settings already there:

```json
{
  "experimental": {
    "openTelemetry": true
  }
}
```

Then set these variables in the terminal where you run `opencode`:

```sh
export OTEL_EXPORTER_OTLP_ENDPOINT="<address>"
export OTEL_EXPORTER_OTLP_HEADERS="Authorization=Bearer <key>"
export OTEL_BSP_SCHEDULE_DELAY=1000

opencode run "Reply with exactly: Lens telemetry check."
```

Use the address and key from the [table above](#address-and-key). OpenCode adds `/v1/traces` to the address. If `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` is already set in that terminal it takes priority and receives the key in `OTEL_EXPORTER_OTLP_HEADERS`, so unset it first

### What Lens shows on 1.x

A session is one trace, and each prompt you send is an `opencode.turn` step in it that shows what you asked and holds the model calls and tool calls made to answer it. Each model call appears once, as an `ai.streamText.doStream` step inside an `ai.streamText` step, with the messages the request carried as its input and the model's reply as its output, including the tool calls it asked for. Each tool call appears as its own step with its input and output. The call OpenCode makes to name a session sits outside the turns. OpenCode applies no size limit to exported content unless you set `OTEL_ATTRIBUTE_VALUE_LENGTH_LIMIT` or `OTEL_SPAN_ATTRIBUTE_VALUE_LENGTH_LIMIT`, so leave both unset

OpenCode does not send its remaining spans when it exits. `OTEL_BSP_SCHEDULE_DELAY=1000` sends them every second instead of every five; without it a short `opencode run` records no model calls. The last model call of a one-shot `opencode run` can still be missing. An interactive session keeps running and sends every call

To stop the export, remove `experimental.openTelemetry`

Verified with OpenCode 1.19.6
