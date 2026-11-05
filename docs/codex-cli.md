# Codex CLI telemetry

Codex sends trace spans and log records through separate OTLP exporters. Configure both endpoints with a Lens tracing key generated for the same Lens workspace

```toml
[otel]
environment = "staging"
log_user_prompt = true
log_agent_responses = true

exporter = { otlp-http = {
    endpoint = "https://lens.example/v1/logs",
    protocol = "json",
    headers = { Authorization = "Bearer <lens-tracing-key>" }
}}
trace_exporter = { otlp-http = {
    endpoint = "https://lens.example/v1/traces",
    protocol = "json",
    headers = { Authorization = "Bearer <lens-tracing-key>" }
}}

[otel.span_attributes]
"gen_ai.agent.name" = "codex-cli"
```

Replace the endpoint with a Lens URL reachable from Codex. Keep the tracing key private and separate from model-provider credentials. Prompt and response logging can capture sensitive content, so enable only the records you intend to retain

Codex supplies `service.name` as resource metadata. Do not override it in `span_attributes`, which adds span-level data; use `gen_ai.agent.name` there when you need an agent label

Lens correlates Codex prompt, response, tool execution, and API request records across both OTLP streams into unified turn traces in the Traces view. Both terminal Codex CLI (`codex_cli_rs`) and IDE integrations powered by `codex-app-server` (such as Xcode) are supported. When Codex sends requests through LiteLLM Gateway, Lens enriches turn and LLM spans with full message history and completion payloads from `spend_logs`. For IDE sessions where clients do not forward trace headers, Lens correlates assistant completions using model and timestamp window matching, while filtering internal lifecycle noise like auth and initialization spans. Both `exporter` and `trace_exporter` may point to `/v1/logs` and `/v1/traces` respectively, or both may point to `/v1/traces`

In Lens, open the trace workspace to inspect Codex turns, or select **Show log events** to inspect raw OTLP log records. Logs with a non-empty trace ID also appear in that trace's detail view. The `/lens/logs` read endpoint supports time, service, event, severity, trace ID, and conversation ID filters

To check for raw prompt log records independently of the trace view, query the log reader with the same Lens tracing key:

```sh
curl --fail-with-body --silent --show-error --get "$LENS_URL/lens/logs" \
  -H "Authorization: Bearer $LENS_TRACE_KEY" \
  --data-urlencode "start_ms=$(($(date +%s%3N) - 86400000))" \
  --data-urlencode "end_ms=$(date +%s%3N)" \
  --data-urlencode "event=codex.user_prompt"
```

An empty `data` array means no matching prompt records are available in that time range. Check the configured log exporter endpoint, the time range, and the Lens version receiving the request before changing trace settings
