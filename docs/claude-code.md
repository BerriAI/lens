# Claude Code telemetry

Claude Code's telemetry is off by default, and Lens needs it to record Claude Code turns. Turn it on in every environment where you run `claude`. Without it Lens only sees the model calls that pass through your LiteLLM gateway and lists them as separate calls, with no turn, tool or subagent steps

Claude Code sends its telemetry to your LiteLLM gateway with the same gateway key it uses for model calls. The gateway forwards it to Lens. You do not put a Lens address or a Lens tracing key on the machine that runs Claude Code. The gateway must already be [connected to Lens](https://docs.litellm.ai/docs/proxy/lens/deployment/litellm)

## Required settings

Set these in the terminal where you run `claude`. Replace the gateway address and key with your own:

```sh
export ANTHROPIC_BASE_URL="http://localhost:4000"
export ANTHROPIC_AUTH_TOKEN="<your LiteLLM key>"

export CLAUDE_CODE_ENABLE_TELEMETRY=1
export CLAUDE_CODE_ENHANCED_TELEMETRY_BETA=1
export CLAUDE_CODE_PROPAGATE_TRACEPARENT=1
export OTEL_TRACES_EXPORTER=otlp
export OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf
export OTEL_EXPORTER_OTLP_TRACES_ENDPOINT="$ANTHROPIC_BASE_URL/v1/traces"
export OTEL_EXPORTER_OTLP_HEADERS="Authorization=Bearer $ANTHROPIC_AUTH_TOKEN"
export OTEL_LOG_USER_PROMPTS=1

claude -p "Reply with exactly: Lens telemetry check."
```

Every one of the telemetry variables is required. This is what each one does and what Lens shows when it is missing:

| Variable | What it does | Without it |
| --- | --- | --- |
| `CLAUDE_CODE_ENABLE_TELEMETRY=1` | Turns Claude Code telemetry on | No turn. Lens lists the gateway's model calls one by one |
| `CLAUDE_CODE_ENHANCED_TELEMETRY_BETA=1` | Turns on the trace exporter that sends turn, tool and subagent spans. The enable flag alone does not | No turn |
| `OTEL_TRACES_EXPORTER=otlp` | Sends those spans over OTLP | No turn |
| `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` | Where the spans go: your gateway address followed by `/v1/traces` | The exporter targets `localhost:4318` and nothing reaches your gateway |
| `OTEL_EXPORTER_OTLP_HEADERS` | Authenticates the upload with your LiteLLM key | The gateway rejects the upload |
| `OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf` | Uses the OTLP/HTTP format the gateway accepts | Claude Code's default export protocol is not guaranteed to match |
| `CLAUDE_CODE_PROPAGATE_TRACEPARENT=1` | Puts the turn's trace ID on each model call | The turn and its gateway calls show up as two unrelated traces |
| `OTEL_LOG_USER_PROMPTS=1` | Includes the text you typed | The turn's input reads `<REDACTED>` |

`OTEL_LOG_USER_PROMPTS=1` stores the prompts you type in Lens. They can contain source code or secrets, so use it only with a Lens deployment where you intend to keep that content

To keep the settings across terminals, put the same telemetry variables in the `env` object of your user-level `~/.claude/settings.json` and keep any settings already there:

```json
{
  "env": {
    "CLAUDE_CODE_ENABLE_TELEMETRY": "1",
    "CLAUDE_CODE_ENHANCED_TELEMETRY_BETA": "1",
    "CLAUDE_CODE_PROPAGATE_TRACEPARENT": "1",
    "OTEL_TRACES_EXPORTER": "otlp",
    "OTEL_EXPORTER_OTLP_PROTOCOL": "http/protobuf",
    "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT": "http://localhost:4000/v1/traces",
    "OTEL_EXPORTER_OTLP_HEADERS": "Authorization=Bearer <your LiteLLM key>",
    "OTEL_LOG_USER_PROMPTS": "1"
  }
}
```

## Check the result

Run one task, then open **Lens > Traces** in your LiteLLM dashboard, or **Traces** in standalone Lens, and filter by **claude-code**. A working setup shows one `claude_code.interaction` trace per turn, with the model calls and tool calls nested under it, the prompt you typed and the cost from the gateway

![A Claude Code turn in Lens with its model calls and a Read tool call](images/claude-code-turn.png)

If you see only rows named `chat <model>` with no `claude_code.interaction` row, the telemetry settings did not reach the `claude` process. Run `env | grep -E 'CLAUDE_CODE|OTEL'` in the same terminal and compare the output with the block above

Verified with Claude Code 2.1.330
