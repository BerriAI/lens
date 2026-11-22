# OpenCode telemetry

OpenCode sends no telemetry by default, and Lens needs it to record OpenCode sessions. What to set depends on the version `opencode --version` prints

OpenCode 2 needs only the environment variables below. It ignores `experimental.openTelemetry`, so skip the config change

On OpenCode 1.x, also turn on its OpenTelemetry spans in `opencode.json`, in your project or in `~/.config/opencode/opencode.json`, and keep any settings already there:

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

The address and key depend on how you run Lens:

| Setup | `<address>` | `<key>` |
| --- | --- | --- |
| OpenCode routed through a LiteLLM gateway [connected to Lens](https://docs.litellm.ai/docs/proxy/lens/deployment/litellm) | Your gateway address, such as `http://localhost:4000` | The LiteLLM key OpenCode already uses for model calls |
| Standalone Lens | Your Lens address, such as `http://localhost:4318` | A Lens tracing key |

OpenCode adds `/v1/traces` to the address. If `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` is already set in that terminal it takes priority and receives the key in `OTEL_EXPORTER_OTLP_HEADERS`, so unset it first

## What Lens shows

A session is one trace, and each prompt you send is an `opencode.turn` step in it that holds the model calls and tool calls made to answer it

With OpenCode 1.x the turn shows what you asked. Each model call appears once, as an `ai.streamText.doStream` step inside an `ai.streamText` step, with the messages the request carried as its input and the model's reply as its output, including the tool calls it asked for. Each tool call appears as its own step with its input and output. The call OpenCode makes to name a session sits outside the turns. OpenCode applies no size limit to exported content unless you set `OTEL_ATTRIBUTE_VALUE_LENGTH_LIMIT` or `OTEL_SPAN_ATTRIBUTE_VALUE_LENGTH_LIMIT`, so leave both unset

OpenCode 2 exports no content. Each model call appears as a `SessionStep.attempt` step with its model and tokens, and each tool call as a step named after the tool, but what you asked, the messages, the replies and the tool input and output are blank. When OpenCode's model calls go through a LiteLLM gateway connected to Lens, each call's cost is the amount the gateway recorded for it, matched through the `traceparent` header OpenCode 2 sends with the request

OpenCode 1.x does not send its remaining spans when it exits. `OTEL_BSP_SCHEDULE_DELAY=1000` sends them every second instead of every five; without it a short `opencode run` records no model calls. The last model call of a one-shot `opencode run` can still be missing. An interactive session keeps running and sends every call

With OpenCode 1.x these settings store what you type, what the model replies, the files OpenCode reads and the output of the commands it runs. That content can include source code or secrets, so use them only with a Lens deployment where you intend to keep it. To stop the export, remove `experimental.openTelemetry`

Verified with OpenCode 1.19.6 and 2.3.2
