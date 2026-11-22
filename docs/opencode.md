# OpenCode telemetry

Lens records both OpenCode 2 and OpenCode 1 sessions. Set the OTLP variables in the terminal where you run `opencode`:

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

## OpenCode 2

OpenCode 2 exports its Effect spans with those variables alone. It ignores `experimental.openTelemetry`, so there is no config change to make. Lens reads each `SessionStep.attempt` as one model call and each `Tool.execute` as one tool call, using the model, token counts, agent name, tool name and tool call id that OpenCode records

OpenCode 2 does not export prompts, replies, tool arguments or tool output, so those steps have no input or output in Lens. The exported data includes session ids, agent names, model names, token counts and tool names. `OTEL_BSP_SCHEDULE_DELAY=1000` is still worth setting because a short `opencode run` can otherwise exit before its last spans are sent

Verified with OpenCode 2.3.2

## OpenCode 1

OpenCode 1 needs the AI SDK instrumentation enabled in `opencode.json`, in your project or in `~/.config/opencode/opencode.json`. Keep any settings already there:

```json
{
  "experimental": {
    "openTelemetry": true
  }
}
```

A session is one trace, and each prompt you send is an `opencode.turn` step in it that shows what you asked and holds the model calls and tool calls made to answer it. Each model call appears once, as an `ai.streamText.doStream` step inside an `ai.streamText` step, with the messages the request carried as its input and the model's reply as its output, including the tool calls it asked for. Each tool call appears as its own step with its input and output. The call OpenCode makes to name a session sits outside the turns. OpenCode applies no size limit to exported content unless you set `OTEL_ATTRIBUTE_VALUE_LENGTH_LIMIT` or `OTEL_SPAN_ATTRIBUTE_VALUE_LENGTH_LIMIT`, so leave both unset

OpenCode 1 sends what you type, what the model replies, the files it reads and the output of the commands it runs. That content can include source code or secrets, so use it only with a Lens deployment where you intend to keep it. To stop the export, remove `experimental.openTelemetry`

Verified with OpenCode 1.19.6
