# Codex CLI telemetry

Codex sends no telemetry by default, and Lens needs it to record Codex turns. Codex sends trace spans and log records through two separate OTLP exporters. Add both to `~/.codex/config.toml` and keep any settings already there:

```toml
[otel]
log_user_prompt = true

exporter = { otlp-http = {
    endpoint = "<address>/v1/logs",
    protocol = "json",
    headers = { Authorization = "Bearer <key>" }
}}
trace_exporter = { otlp-http = {
    endpoint = "<address>/v1/traces",
    protocol = "json",
    headers = { Authorization = "Bearer <key>" }
}}
```

The address and key depend on how you run Lens:

| Setup | `<address>` | `<key>` |
| --- | --- | --- |
| Codex routed through a LiteLLM gateway [connected to Lens](https://docs.litellm.ai/docs/proxy/lens/deployment/litellm) | Your gateway address, such as `http://localhost:4000` | The LiteLLM key Codex already uses for model calls |
| Standalone Lens | Your Lens address, such as `http://localhost:4318` | A Lens tracing key |

## What Lens shows

A Codex conversation appears as one trace, grouped by the `conversation.id` Codex stamps on its telemetry. Each turn in it shows the prompt you typed, each tool call with its arguments and output, and the model calls with their duration and token counts. Resuming a conversation adds its new turns to the same trace. Codex puts `conversation.id` on only some of its spans and exports them in batches a few seconds apart, so a model call exported in a batch with none of those spans appears as its own small trace instead of joining the conversation. Model call steps have no input or output: Codex exports neither the request it sent nor the model's reply. `log_agent_responses = true` is accepted and sends nothing

Codex cuts each tool output to about 2,000 characters before it exports it and ends it with `[... telemetry preview truncated ...]`. Codex has no setting that raises this limit

`log_user_prompt = true` stores what you type, and tool steps store the arguments and output of the commands Codex runs, which can include file contents or secrets. Use these settings only with a Lens deployment where you intend to keep that content.

Codex supplies `service.name` as resource metadata. To label the agent, add `"gen_ai.agent.name" = "codex-cli"` under `[otel.span_attributes]`; do not override `service.name` there. Both terminal Codex CLI (`codex_cli_rs`) and IDE integrations powered by `codex-app-server` (such as Xcode) are supported. Internal lifecycle spans such as auth and initialization are filtered from the trace view

In Lens, open the trace workspace to inspect Codex turns, or select **Show log events** to inspect raw OTLP log records. Logs with a non-empty trace ID also appear in that trace's detail view. The `/lens/logs` read endpoint supports time, service, event, severity, trace ID, and conversation ID filters

To check for raw prompt log records independently of the trace view, query the log reader with the same Lens tracing key:

```sh
curl --fail-with-body --silent --show-error --get "$LENS_URL/lens/logs" \
  -H "Authorization: Bearer $LENS_TRACE_KEY" \
  --data-urlencode "start_ms=$((($(date +%s) - 86400) * 1000))" \
  --data-urlencode "end_ms=$(($(date +%s) * 1000))" \
  --data-urlencode "event=codex.user_prompt"
```

An empty `data` array means no matching prompt records are available in that time range. Check the configured log exporter endpoint, the time range, and the Lens version receiving the request before changing trace settings

Verified with Codex CLI 0.171.0
