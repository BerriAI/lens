import { describe, expect, it } from "vitest";

import research from "../../__fixtures__/research_trace.json";
import { type SessionTurn, turnOpeningCall } from "../../turns";
import type { Span, SpanDetail, TraceMessage } from "../../types";
import { turnFacts } from "./SpanHoverCard";
import { turnTitle } from "./turnTitles";

const base = { ...research.spans[0], parent_span_id: null, status: "ok", error: null } as Span;
const call = (id: string, start: number, duration: number, inputTokens: number): Span => ({
  ...base,
  span_id: id,
  type: "llm",
  start_offset_ms: start,
  duration_ms: duration,
  input_tokens: inputTokens,
});
const turn = (spans: Span[]): SessionTurn => ({ id: "turn::a", index: 1, spans });
const request = (messages: TraceMessage[]): SpanDetail => ({
  span_id: "a",
  input: JSON.stringify(messages),
  output: "",
  attributes: {},
});
const REMINDER = "<system-reminder>\nContents of AGENTS.md\n</system-reminder>";

describe("turnTitle", () => {
  it("is the message typed after the last assistant message, without the blocks the harness added", () => {
    const detail = request([
      { role: "system", content: "You are a coding agent" },
      { role: "user", content: `${REMINDER}\n\nfirst question` },
      { role: "assistant", content: "first answer" },
      {
        role: "user",
        content: `${REMINDER}\n<system-reminder>git status</system-reminder>\n\nrun ls and\ncount the files`,
      },
    ]);
    expect(turnTitle(detail)).toBe("run ls and count the files");
  });

  it("drops a slash command's wrappers and keeps what was typed after it", () => {
    const content =
      "<local-command-caveat>The command below was run directly</local-command-caveat>\n" +
      "<command-name>/compact</command-name>\n<command-message>compact</command-message>\n<command-args></command-args>\n" +
      "<local-command-stdout>Compacted</local-command-stdout>\n\nwhat is 9 plus 4?";
    expect(
      turnTitle(
        request([
          { role: "user", content: REMINDER },
          { role: "user", content },
        ]),
      ),
    ).toBe("what is 9 plus 4?");
  });

  it("keeps text that only looks like a block because its tag never closes", () => {
    expect(turnTitle(request([{ role: "user", content: "why is <system-reminder> in my trace" }]))).toBe(
      "why is <system-reminder> in my trace",
    );
  });

  it("is empty when the request adds only harness text or tool results", () => {
    expect(turnTitle(request([{ role: "user", content: REMINDER }]))).toBe("");
    expect(
      turnTitle(
        request([
          { role: "user", content: "question" },
          { role: "assistant", content: "" },
          { role: "user", content: "" },
        ]),
      ),
    ).toBe("");
  });
});

describe("turnOpeningCall", () => {
  it("skips a smaller side request that started first and is still running", () => {
    const spans = [call("title", 0, 900, 800), call("main", 176, 2100, 19000), call("later", 5000, 500, 25000)];
    expect(turnOpeningCall(turn(spans))?.span_id).toBe("main");
  });

  it("is the first model call when nothing overlaps it, ignoring subagent calls", () => {
    const sub = { ...call("sub", 10, 50, 90000), parent_span_id: "agent" };
    expect(turnOpeningCall(turn([call("main", 0, 100, 100), sub, call("next", 200, 100, 30000)]))?.span_id).toBe(
      "main",
    );
    expect(turnOpeningCall(turn([{ ...base, type: "tool" }]))).toBeUndefined();
  });
});

describe("turn hover", () => {
  it("shows the typed message as the turn's input", () => {
    const row = {
      kind: "turn" as const,
      id: "turn::a",
      depth: 0,
      index: 2,
      members: [call("m", 0, 10, 5)],
      collapsed: false,
    };
    expect(turnFacts(row, "run ls").input).toBe("run ls");
    expect(turnFacts(row).input).toBe("");
  });
});
