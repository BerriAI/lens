import { describe, expect, it } from "vitest";

import research from "./__fixtures__/research_trace.json";
import type { SpanTreeState } from "./tree";
import { sessionTree } from "./turns";
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
const PROMPT = "List all files in the project folder with ls and report the names only";

const call = (id: string, start: number, duration: number, preview = FIRST): Span => ({
  ...base,
  span_id: id,
  type: "llm",
  name: "chat model",
  start_offset_ms: start,
  duration_ms: duration,
  input_preview: preview,
});
const tool = (id: string, name: string, start: number, preview: string): Span => ({
  ...base,
  span_id: id,
  type: "tool",
  name: `execute_tool ${name}`,
  start_offset_ms: start,
  duration_ms: 0,
  input_preview: preview,
});
const agentArgs = (prompt: string): string => JSON.stringify({ description: "List files", prompt });

/** Timings of a real Claude Code session through the gateway: a Read turn, then a turn that launches a subagent and answers once it reports back. */
const session = (launch = agentArgs(PROMPT)): Span[] => [
  call("m1", 0, 1307),
  tool("read", "Read", 1482, '{"file_path": "/proj/AGENTS.md"}'),
  call("m2", 1496, 701),
  call("m3", 17395, 2764, SECOND),
  call("s1", 20334, 1304, PROMPT),
  tool("agent", "Agent", 20348, launch),
  call("m4", 20362, 910, SECOND),
  tool("bash", "Bash", 21727, '{"command": "ls -la"}'),
  call("s2", 21735, 1928, PROMPT),
  call("m5", 23833, 1528, SECOND),
];

const outline = (spans: readonly Span[], state = STATE): string[] =>
  buildTreeRows(spans, state).map(
    (row) => `${"  ".repeat(row.depth)}${row.kind === "turn" ? `Turn ${row.index}` : row.id}`,
  );

describe("coding session turns", () => {
  it("files a gateway session's flat calls under turns, with subagent calls under their Agent call", () => {
    expect(outline(session().toReversed())).toEqual([
      "Turn 1",
      "  m1",
      "  read",
      "  m2",
      "Turn 2",
      "  m3",
      "  agent",
      "    s1",
      "    bash",
      "    s2",
      "  m4",
      "  m5",
    ]);
  });

  it("opens a turn only when the user types a new message, however many calls and tools answer it", () => {
    const spans = [
      { ...call("title", 0, 900, "write a title"), model: "claude-haiku-4-5" },
      call("m1", 176, 2100),
      call("m2", 30000, 2900, SECOND),
      tool("search", "ToolSearch", 33000, '{"query": "fetch web page"}'),
      call("m3", 33000, 1700, SECOND),
      call("notified", 90000, 1500, SECOND),
      call("m4", 120000, 800, "thanks"),
    ];
    const turns = buildTreeRows(spans, STATE).flatMap((row) => (row.kind === "turn" ? [row] : []));
    expect(turns.map((turn) => [turn.message, turn.members.map((span) => span.span_id)])).toEqual([
      [FIRST, ["title", "m1"]],
      [SECOND, ["m2", "search", "m3", "notified"]],
      ["thanks", ["m4"]],
    ]);
  });

  it("splits calls that carry no content where one finishes before the next starts", () => {
    const spans = [
      call("m1", 0, 100, ""),
      tool("read", "Read", 150, "{}"),
      call("m2", 150, 100, ""),
      call("m3", 900, 100, ""),
    ];
    expect(outline(spans)).toEqual(["Turn 1", "  m1", "  read", "  m2", "Turn 2", "  m3"]);
  });

  it("links a subagent whose Agent arguments were cut short in the preview", () => {
    const cut = agentArgs(PROMPT).slice(0, -30);
    expect(sessionTree(session(cut))?.spans.flatMap((span) => (span.parent_span_id ? [span.span_id] : []))).toEqual([
      "s1",
      "bash",
      "s2",
    ]);
  });

  it("keeps a call that only shares the first words of an Agent prompt in the main thread", () => {
    const spans = session().map((span) =>
      span.span_id.startsWith("s") ? { ...span, input_preview: "List all" } : span,
    );
    expect(sessionTree(spans)?.spans.every((span) => span.parent_span_id === null)).toBe(true);
  });

  it("keeps the launching thread out of the branch when the subagent is given the user's own message", () => {
    const spans = session(agentArgs(FIRST));
    expect(sessionTree(spans)?.spans.every((span) => span.parent_span_id === null)).toBe(true);
  });

  it("does not open a turn for a call that starts while another is still running", () => {
    const spans = [call("title", 0, 900, "write a title"), call("m1", 176, 2100), call("m2", 5000, 800, SECOND)];
    expect(outline(spans)).toEqual(["Turn 1", "  title", "  m1", "Turn 2", "  m2"]);
  });

  it("leaves traces with a span tree, or a single model call, without turns", () => {
    expect(sessionTree(research.spans as Span[])).toBeNull();
    expect(sessionTree([call("only", 0, 10), tool("t", "Read", 20, "{}")])).toBeNull();
    expect(buildTreeRows(research.spans as Span[], STATE).some((row) => row.kind === "turn")).toBe(false);
  });

  it("folds a collapsed turn and opens it again to reveal a step inside a subagent", () => {
    const spans = session();
    const turn = buildTreeRows(spans, STATE).find((row) => row.kind === "turn" && row.index === 2);
    const folded = {
      ...STATE,
      collapsedSpanIds: new Set([turn?.id ?? "", "agent"]),
    };
    expect(outline(spans, folded)).toEqual(["Turn 1", "  m1", "  read", "  m2", "Turn 2"]);
    expect(outline(spans, revealSpanInState(spans, folded, "bash"))).toContain("    bash");
  });

  it("totals a turn over its own calls and its subagent's", () => {
    const row = buildTreeRows(session(), STATE).find((candidate) => candidate.kind === "turn" && candidate.index === 2);
    expect(row?.kind === "turn" && row.members.map((span) => span.span_id)).toEqual([
      "m3",
      "agent",
      "m4",
      "m5",
      "s1",
      "bash",
      "s2",
    ]);
  });
});
