const SETTINGS = [
  "CLAUDE_CODE_ENABLE_TELEMETRY=1",
  "CLAUDE_CODE_ENHANCED_TELEMETRY_BETA=1",
  "CLAUDE_CODE_PROPAGATE_TRACEPARENT=1",
  "OTEL_TRACES_EXPORTER=otlp",
  "OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf",
  "OTEL_EXPORTER_OTLP_ENDPOINT=<your LiteLLM gateway URL>",
  'OTEL_EXPORTER_OTLP_HEADERS="Authorization=Bearer <your LiteLLM key>"',
  "OTEL_LOG_USER_PROMPTS=1",
];

/** Shown on a Claude Code session the gateway recorded on its own: the calls are listed in order, with no turns. */
export function TelemetryNotice() {
  return (
    <div className="border-b bg-muted/40 px-4 py-2 text-xs text-muted-foreground" role="note">
      <p>
        Claude Code telemetry was off for this session, so Lens lists its model and tool calls in order and cannot show
        which message each one answers. To see each message you type as a numbered turn, set these in the{" "}
        <code className="font-mono">env</code> of <code className="font-mono">~/.claude/settings.json</code> and start a
        new session:
      </p>
      <pre className="mt-1.5 overflow-x-auto font-mono text-[11px] text-foreground">{SETTINGS.join("\n")}</pre>
    </div>
  );
}
