import { describe, expect, it } from "vitest";

import type { Span } from "../../traces/types";
import { compareSteps, toolSteps } from "./steps";

const span = (span_id: string, name: string, overrides: Partial<Span> = {}): Span => ({
  span_id,
  parent_span_id: "root",
  name,
  type: "tool",
  agent: "refund-agent",
  framework: "",
  start_offset_ms: 0,
  duration_ms: 10,
  status: "ok",
  error: null,
  error_truncated: false,
  input_preview: "",
  model: null,
  input_tokens: 0,
  output_tokens: 0,
  litellm_request_id: null,
  spend: null,
  spend_log_request_id: null,
  spend_match: null,
  ...overrides,
});

const changes = (steps: readonly { span: Span; change: string }[]) =>
  steps.map(({ span: step, change }) => `${step.name}:${change}`);

describe("toolSteps", () => {
  it("keeps only tool spans, ordered by start time", () => {
    const spans = [
      span("late", "issue_refund", { start_offset_ms: 30 }),
      span("llm", "chat", { type: "llm", start_offset_ms: 5 }),
      span("early", "lookup_order", { start_offset_ms: 10 }),
    ];

    expect(toolSteps(spans).map((step) => step.span_id)).toEqual(["early", "late"]);
  });
});

describe("compareSteps", () => {
  it.each([
    {
      name: "a dropped check is skipped on the baseline side",
      baseline: ["lookup_order", "check_policy", "issue_refund"],
      candidate: ["lookup_order", "issue_refund"],
      expected: {
        baseline: ["lookup_order:same", "check_policy:skipped", "issue_refund:same"],
        candidate: ["lookup_order:same", "issue_refund:same"],
      },
    },
    {
      name: "a new call is added on the candidate side",
      baseline: ["lookup_order"],
      candidate: ["lookup_order", "send_email"],
      expected: { baseline: ["lookup_order:same"], candidate: ["lookup_order:same", "send_email:added"] },
    },
    {
      name: "reordered calls keep the longest common run and flag the rest",
      baseline: ["a", "b", "c"],
      candidate: ["c", "a", "b"],
      expected: { baseline: ["a:same", "b:same", "c:skipped"], candidate: ["c:added", "a:same", "b:same"] },
    },
    {
      name: "identical runs have no changes",
      baseline: ["a", "a"],
      candidate: ["a", "a"],
      expected: { baseline: ["a:same", "a:same"], candidate: ["a:same", "a:same"] },
    },
    {
      name: "an empty candidate skips every baseline step",
      baseline: ["a", "b"],
      candidate: [],
      expected: { baseline: ["a:skipped", "b:skipped"], candidate: [] },
    },
  ])("$name", ({ baseline, candidate, expected }) => {
    const compared = compareSteps(
      baseline.map((name, i) => span(`b${i}`, name)),
      candidate.map((name, i) => span(`c${i}`, name)),
    );

    expect({ baseline: changes(compared.baseline), candidate: changes(compared.candidate) }).toEqual(expected);
  });
});
