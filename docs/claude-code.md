# Claude Code

Lens records Claude Code through the LiteLLM Lens plugin for Claude Code. The plugin is the recommended setup: every Claude Code session becomes exactly one trace. Claude Code's own telemetry remains supported as a [fallback](#fallback-claude-code-telemetry) for machines that cannot run the plugin

## Install the plugin

The plugin needs Claude Code, `python3` on the PATH, and macOS or Linux

```sh
claude plugin marketplace add BerriAI/litellm-lens-plugins
claude plugin install litellm-lens@berriai-lens \
  --config lens_url=<address> --config lens_key="$<variable holding the key>"
```

Leave out the two `--config` options and Claude Code asks for the address and key the next time it starts. Installing again does not change a saved address or key, so run `claude plugin uninstall litellm-lens@berriai-lens` first to change them. The quickest way to do all of this is the setup prompt on the Lens home page: copy it and give it to Claude Code

| Setup | `<address>` | Key |
| --- | --- | --- |
| Claude Code routed through a LiteLLM gateway [connected to Lens](https://docs.litellm.ai/docs/proxy/lens/deployment/litellm) | Your gateway address, such as `http://localhost:4000` | The LiteLLM key Claude Code already uses for model calls, usually in `ANTHROPIC_AUTH_TOKEN` |
| Standalone Lens | Your Lens address, such as `http://localhost:4318` | A Lens tracing key |

Through a gateway, the gateway stamps each trace with the key's user and team and forwards it to Lens

## What Lens shows with the plugin

One trace per Claude Code session. Each prompt is a turn with its reply, each tool call is a step with its arguments and output, including failed tools, and each model call is its own step with its model, tokens and reply. A subagent's steps nest under the turn that started it. When Claude Code's model calls go through the LiteLLM gateway, each model-call step also shows its cost, matched to the gateway's spend record by response ID

The plugin reads the transcript Claude Code writes locally after each turn and sends it in the background, so a trace updates when a turn completes. A model-call step shows the messages added since the previous call, not the full request. Thinking, images and documents are not sent. If no trace arrives, read `last_error.txt` in `~/.claude/plugins/data/litellm-lens-berriai-lens`

The plugin stores what you type, the replies, and the arguments and output of the tools Claude Code runs, which can include file contents or secrets. Use it only with a Lens deployment where you intend to keep that content. See the [plugin README](https://github.com/BerriAI/litellm-lens-plugins/tree/main/claude-code) for details

Verified with Claude Code 2.1.330 on Linux

## Fallback: Claude Code telemetry

Use this when the plugin cannot run, for example on Windows or without Python. The plugin sends nothing while `CLAUDE_CODE_ENABLE_TELEMETRY` and `CLAUDE_CODE_ENHANCED_TELEMETRY_BETA` are both set, so a session is never recorded twice; remove those two settings to use the plugin. Without the plugin or telemetry, Lens only sees the model calls that pass through your LiteLLM gateway and lists them as separate calls, with no turn, tool or subagent steps

Claude Code routed through a LiteLLM gateway sends its telemetry to that gateway with the same gateway key it uses for model calls. The gateway forwards it to Lens, so you do not put a Lens address or a Lens tracing key on the machine that runs Claude Code. Claude Code that does not use LiteLLM sends the same telemetry [straight to Lens](#standalone-lens) with a Lens tracing key

### Required settings

Run this in the terminal where you run `claude` and replace the gateway address and key with your own:

```sh
export ANTHROPIC_BASE_URL="http://localhost:4000"
export ANTHROPIC_AUTH_TOKEN="<your LiteLLM key>"

export CLAUDE_CODE_ENABLE_TELEMETRY=1
export CLAUDE_CODE_ENHANCED_TELEMETRY_BETA=1
export CLAUDE_CODE_PROPAGATE_TRACEPARENT=1
export OTEL_TRACES_EXPORTER=otlp
export OTEL_LOGS_EXPORTER=otlp
export OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf
export OTEL_EXPORTER_OTLP_ENDPOINT="$ANTHROPIC_BASE_URL"
export OTEL_EXPORTER_OTLP_HEADERS="Authorization=Bearer $ANTHROPIC_AUTH_TOKEN"
export OTEL_LOG_USER_PROMPTS=1
export OTEL_LOG_ASSISTANT_RESPONSES=1
export OTEL_LOG_TOOL_DETAILS=1
export OTEL_LOG_TOOL_CONTENT=1
export OTEL_LOG_RAW_API_BODIES=1
export CLAUDE_CODE_OTEL_CONTENT_MAX_LENGTH=16777216

claude -p "Reply with exactly: Lens telemetry check."
```

This is what each telemetry variable does and what Lens shows when it is missing:

| Variable | What it does | Without it |
| --- | --- | --- |
| `CLAUDE_CODE_ENABLE_TELEMETRY=1` | Turns Claude Code telemetry on | No turn. Lens lists the gateway's model calls one by one |
| `CLAUDE_CODE_ENHANCED_TELEMETRY_BETA=1` | Turns on the trace exporter that sends turn, tool and subagent spans. The enable flag alone does not | No turn |
| `OTEL_TRACES_EXPORTER=otlp` | Sends those spans over OTLP | No turn |
| `OTEL_LOGS_EXPORTER=otlp` | Sends Claude Code's events over OTLP: assistant replies, tool results and model call bodies | Steps have no replies, tool output or model call content |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Where the spans and events go: your gateway address. Claude Code adds `/v1/traces` and `/v1/logs` | The exporter targets `localhost:4318` and nothing reaches your gateway |
| `OTEL_EXPORTER_OTLP_HEADERS` | Authenticates the upload with your LiteLLM key | The gateway rejects the upload |
| `OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf` | Uses the OTLP/HTTP format the gateway accepts | Claude Code's default export protocol is not guaranteed to match |
| `CLAUDE_CODE_PROPAGATE_TRACEPARENT=1` | Puts the turn's trace ID on each model call | The turn and its gateway calls show up as two unrelated traces |
| `OTEL_LOG_USER_PROMPTS=1` | Includes the text you typed | The turn's input reads `<REDACTED>` |
| `OTEL_LOG_ASSISTANT_RESPONSES=1` | Includes the text Claude Code replied with | The turn has no reply |
| `OTEL_LOG_TOOL_DETAILS=1` and `OTEL_LOG_TOOL_CONTENT=1` | Include each tool call's arguments and output | Tool steps show a name and a duration only |
| `OTEL_LOG_RAW_API_BODIES=1` | Includes the request and response of every model call | A model call step reads "No content recorded for this span" |
| `CLAUDE_CODE_OTEL_CONTENT_MAX_LENGTH=16777216` | Raises Claude Code's 60 KB limit on exported content to 16 MiB | Requests longer than 60 KB arrive cut off, and their model call step has no input |

### Standalone Lens

When Claude Code does not go through a LiteLLM gateway, leave `ANTHROPIC_BASE_URL` and `ANTHROPIC_AUTH_TOKEN` as they are and change only where the telemetry goes. Every other variable stays the same:

| Setup | `OTEL_EXPORTER_OTLP_ENDPOINT` | Key in `OTEL_EXPORTER_OTLP_HEADERS` |
| --- | --- | --- |
| Claude Code routed through a LiteLLM gateway | Your gateway address, such as `http://localhost:4000` | The LiteLLM key Claude Code already uses for model calls |
| Standalone Lens | Your Lens address, such as `http://localhost:4318` | A Lens tracing key |

Lens groups the turns of a session without any resource attribute. The cost of a trace is filled only when the model calls go through a gateway, which is where the price is known

### Model call content

With `OTEL_LOG_RAW_API_BODIES=1`, each model call step shows the conversation the request carried as its input and what the model returned as its output, which is usually the next tool call. Lens keeps the system prompt, the messages and the reply. It does not keep tool definitions or thinking blocks

Each request repeats the conversation so far, so a long session stores it once per model call. A request over the content limit arrives cut off and its step stays empty

These settings store what you type, what Claude Code replies, the files it reads and the output of the commands it runs. That content can include source code or secrets, so use them only with a Lens deployment where you intend to keep it. To leave model call bodies out, set `OTEL_LOG_RAW_API_BODIES=0`

To keep the settings across terminals, put the same telemetry variables in the `env` object of your user-level `~/.claude/settings.json` and keep any settings already there:

```json
{
  "env": {
    "CLAUDE_CODE_ENABLE_TELEMETRY": "1",
    "CLAUDE_CODE_ENHANCED_TELEMETRY_BETA": "1",
    "CLAUDE_CODE_PROPAGATE_TRACEPARENT": "1",
    "OTEL_TRACES_EXPORTER": "otlp",
    "OTEL_LOGS_EXPORTER": "otlp",
    "OTEL_EXPORTER_OTLP_PROTOCOL": "http/protobuf",
    "OTEL_EXPORTER_OTLP_ENDPOINT": "http://localhost:4000",
    "OTEL_EXPORTER_OTLP_HEADERS": "Authorization=Bearer <your LiteLLM key>",
    "OTEL_LOG_USER_PROMPTS": "1",
    "OTEL_LOG_ASSISTANT_RESPONSES": "1",
    "OTEL_LOG_TOOL_DETAILS": "1",
    "OTEL_LOG_TOOL_CONTENT": "1",
    "OTEL_LOG_RAW_API_BODIES": "1",
    "CLAUDE_CODE_OTEL_CONTENT_MAX_LENGTH": "16777216"
  }
}
```

### Check the telemetry result

Run one task, then open **Lens > Traces** in your LiteLLM dashboard, or **Traces** in standalone Lens, and filter by **claude-code**. A working setup shows one `claude_code.interaction` trace per turn, with the model calls and tool calls nested under it, the prompt you typed and the cost from the gateway. Select a model call to see the conversation it received and what the model returned

![A Claude Code turn in Lens with its model calls and a Read tool call](images/claude-code-turn.png)

If you see only rows named `chat <model>` with no `claude_code.interaction` row, the telemetry settings did not reach the `claude` process. Run `env | grep -E 'CLAUDE_CODE|OTEL'` in the same terminal and compare the output with the block above

Verified with Claude Code 2.1.330
