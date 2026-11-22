import { describe, expect, it } from "vitest";

import research from "../../__fixtures__/research_trace.json";
import type { Span } from "../../types";
import { spanFacts, turnFacts } from "./SpanHoverCard";

const base = { ...research.spans[0], parent_span_id: null, status: "ok", error: null } as Span;
const step = (overrides: Partial<Span>): Span => ({ ...base, ...overrides });

describe("hover facts", () => {
  const root = step({ span_id: "turn", type: "agent", start_offset_ms: 100, duration_ms: 100, input_preview: "hi" });

  it("takes a turn's time from its own span and sums tokens, cost and step counts over the calls in it", () => {
    const members = [
      step({ span_id: "own", parent_span_id: "turn", type: "llm", input_tokens: 10, output_tokens: 2, spend: 0.25 }),
      step({ parent_span_id: "own", type: "framework", start_offset_ms: 100, duration_ms: 50, input_tokens: 10, output_tokens: 2, spend: 0.25 }),
      step({ type: "tool", start_offset_ms: 160, duration_ms: 0, input_tokens: 0, output_tokens: 0, spend: null }),
      step({ type: "llm", start_offset_ms: 170, duration_ms: 30, input_tokens: 20, output_tokens: 3, spend: 0.5 }),
    ];
    expect(turnFacts({ kind: "turn", id: "turn::a", depth: 0, index: 4, root, members, collapsed: false })).toMatchObject({
      name: "Turn 4",
      startMs: 100,
      durationMs: 100,
      tokens: 35,
      inputTokens: 30,
      outputTokens: 5,
      spend: 0.75,
      modelCalls: 2,
      toolCalls: 1,
      input: "hi",
      failed: false,
    });
  });

  it("reports a failed turn with the first error among its members", () => {
    const members = [step({ type: "llm" }), step({ type: "tool", status: "error", error: "exit 1" })];
    expect(turnFacts({ kind: "turn", id: "turn::a", depth: 0, index: 1, root, members, collapsed: false })).toMatchObject({
      failed: true,
      error: "exit 1",
    });
  });

  it("gives a step its token split and what it was given", () => {
    const model = step({ type: "llm", input_tokens: 7, output_tokens: 4, input_preview: "what is 17 times 3?" });
    const bash = step({ type: "tool", input_preview: '{"command": "ls -la"}' });
    expect(spanFacts(model)).toMatchObject({ inputTokens: 7, outputTokens: 4, input: "what is 17 times 3?" });
    expect(spanFacts(bash).input).toBe("ls -la");
  });
});
