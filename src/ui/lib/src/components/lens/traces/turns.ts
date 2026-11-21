import type { Span } from "./types";

const NATIVE_AGENT_TOOLS = ["Agent", "Task", "execute_tool Agent", "execute_tool Task"];

export function isNativeAgent(span: Span): boolean {
  return span.framework === "claude-code" && span.type === "tool" && NATIVE_AGENT_TOOLS.includes(span.name);
}

export interface SessionTurn {
  id: string;
  index: number;
  /** The span Claude Code recorded for the typed message; it carries the message and the reply. */
  root: Span;
  /** Everything recorded under the turn, in time order. */
  spans: Span[];
}

const byStart = (a: Span, b: Span): number => a.start_offset_ms - b.start_offset_ms;

const isTurn = (span: Span): boolean =>
  span.parent_span_id === null && span.type === "agent" && span.framework === "claude-code";

/**
 * Claude Code records one parentless agent span per typed message when its telemetry is on, and Lens files
 * a session's spans in one trace, so each of those spans is a turn. A trace without them has no turns.
 */
export function sessionTurns(recorded: readonly Span[]): SessionTurn[] {
  const spans = [...recorded].sort(byStart);
  const under = (id: string): Span[] =>
    spans.filter((span) => span.parent_span_id === id).flatMap((span) => [span, ...under(span.span_id)]);
  return spans.filter(isTurn).map((root, index) => ({
    id: `turn::${root.span_id}`,
    index: index + 1,
    root,
    spans: under(root.span_id).sort(byStart),
  }));
}

/** A gateway records only the model and tool calls of a Claude Code session, each parentless, so nothing says where a turn starts. */
export function lacksAgentTelemetry(spans: readonly Span[]): boolean {
  return (
    spans.some((span) => span.framework === "claude-code") &&
    spans.every((span) => span.parent_span_id === null && (span.type === "llm" || span.type === "tool"))
  );
}

/** The gateway's record of a model call Claude Code also recorded is folded into Claude Code's step, so only that step is counted. */
export function countedOnce(spans: readonly Span[]): Span[] {
  return spans.filter((span) => span.type !== "framework");
}
