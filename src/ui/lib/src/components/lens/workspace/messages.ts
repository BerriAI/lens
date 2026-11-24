import type { AgentEvent } from "../../../lib/http/agentWorkspace";

const text = (value: unknown): string =>
  typeof value === "string" ? value : "";
const object = (value: unknown): Record<string, unknown> =>
  value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};

export function assistantText(events: readonly AgentEvent[]): string {
  const completed = events.findLast(
    (event) => event.kind === "assistant.completed",
  );
  if (completed) return text(completed.data.content);
  return events
    .map((event) => {
      if (event.kind === "assistant.delta") return text(event.data.delta);
      if (
        event.kind === "update" &&
        event.data.sessionUpdate === "agent_message_chunk"
      )
        return text(object(event.data.content).text);
      return "";
    })
    .join("");
}

export function toolActivity(
  events: readonly AgentEvent[],
): { id: string; title: string; status: string }[] {
  const calls = new Map<
    string,
    { id: string; title: string; status: string }
  >();
  for (const event of events) {
    if (
      event.kind === "update" &&
      ["tool_call", "tool_call_update"].includes(text(event.data.sessionUpdate))
    ) {
      const id = text(event.data.toolCallId);
      const previous = calls.get(id);
      calls.set(id, {
        id,
        title: text(event.data.title) || previous?.title || "Using a tool",
        status: text(event.data.status) || previous?.status || "running",
      });
    }
    if (event.kind.startsWith("tool.")) {
      const id = text(event.data.tool_name);
      calls.set(id, { id, title: id, status: event.kind.slice(5) });
    }
  }
  return [...calls.values()];
}
