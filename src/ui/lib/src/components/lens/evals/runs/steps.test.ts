import { describe, expect, it } from "vitest";

import { compareSteps, type ComparedStep } from "./steps";
import { step } from "./testRuns";

const changes = (steps: readonly ComparedStep[]) =>
  steps.map(({ step: s, change }) => `${s.tool_name}:${change}`);
const tools = (...names: string[]) =>
  names.map((name, index) => step(name, index * 10));

describe("compareSteps", () => {
  it.each([
    {
      name: "a dropped check is skipped on the baseline",
      baseline: ["read", "run_tests", "open_pr"],
      candidate: ["read", "open_pr"],
      expected: {
        baseline: ["read:same", "run_tests:skipped", "open_pr:same"],
        candidate: ["read:same", "open_pr:same"],
      },
    },
    {
      name: "a new call is added on the candidate",
      baseline: ["read", "open_pr"],
      candidate: ["read", "search", "open_pr"],
      expected: {
        baseline: ["read:same", "open_pr:same"],
        candidate: ["read:same", "search:added", "open_pr:same"],
      },
    },
    {
      name: "an empty candidate skips every baseline call",
      baseline: ["read", "open_pr"],
      candidate: [],
      expected: {
        baseline: ["read:skipped", "open_pr:skipped"],
        candidate: [],
      },
    },
  ])("$name", ({ baseline, candidate, expected }) => {
    const compared = compareSteps(tools(...baseline), tools(...candidate));
    expect({
      baseline: changes(compared.baseline),
      candidate: changes(compared.candidate),
    }).toEqual(expected);
  });
});
