import { describe, expect, it } from "vitest";

import research from "./__fixtures__/research_trace.json";
import type { SpanTreeState } from "./tree";
import { sessionTurns } from "./turns";
import type { Span } from "./types";
import { buildTreeRows, revealSpanInState } from "./utils";

const STATE: SpanTreeState = {
  hideFramework: true,
  collapsedSpanIds: new Set(),
  expandedGroupIds: new Set(),
  groupRevealCounts: {},
};
const base = {
  ...research.spans[0],
  parent_span_id: null,
  framework: "claude-code",
  status: "ok",
} as Span;
const FIRST = "what is 17 times 3?";
const SECOND = "list the project files with a subagent";

const step = (id: string, parent: string | null, type: string, start: number, fields: Partial<Span> = {}): Span => ({
  ...base,
  span_id: id,
  parent_span_id: parent,
  type,
  name: type === "llm" ? "chat model" : id,
  start_offset_ms: start,
  duration_ms: 10,
  input_preview: "",
  ...fields,
});
const typed = (id: string, start: number, message: string): Span =>
  step(id, null, "agent", start, { name: "claude_code.interaction", input_preview: message, duration_ms: 500 });

/** The shape of a real Claude Code session with telemetry on: one interaction span per typed message, the gateway's call nested under Claude Code's own. */
const session = (): Span[] => [
  typed("t1", 0, FIRST),
  step("r1", "t1", "llm", 10),
  step("g1", "r1", "llm", 12, { input_tokens: 40, output_tokens: 4, spend: 0.5 }),
  step("read", "t1", "tool", 30),
  typed("t2", 1000, SECOND),
  step("r2", "t2", "llm", 1010),
  step("g2", "r2", "llm", 1012, { input_preview: "[SUGGESTION MODE: Suggest what the user might naturally type next]" }),
];

const outline = (spans: readonly Span[], state = STATE): string[] =>
  buildTreeRows(spans, state).map(
    (row) => `${"  ".repeat(row.depth)}${row.kind === "turn" ? `Turn ${row.index}` : row.id}`,
  );

describe("Claude Code turns", () => {
  it("numbers the messages Claude Code recorded and files what it did for each under that turn", () => {
    expect(outline(session().toReversed())).toEqual([
      "Turn 1",
      "  r1",
      "    g1",
      "  read",
      "Turn 2",
      "  r2",
      "    g2",
    ]);
  });

  it("labels a turn with the message Claude Code recorded, whatever its model calls were sent", () => {
    expect(sessionTurns(session()).map((turn) => turn.root.input_preview)).toEqual([FIRST, SECOND]);
  });

  it("gives a trace that is not a Claude Code session no turns", () => {
    expect(sessionTurns(research.spans as Span[])).toEqual([]);
  });

  it("folds a collapsed turn and opens it again to reveal a step inside it", () => {
    const spans = session();
    const folded = { ...STATE, collapsedSpanIds: new Set(["turn::t2"]) };
    expect(outline(spans, folded)).toEqual(["Turn 1", "  r1", "    g1", "  read", "Turn 2"]);
    expect(outline(spans, revealSpanInState(spans, folded, "g2"))).toContain("    g2");
  });
});
