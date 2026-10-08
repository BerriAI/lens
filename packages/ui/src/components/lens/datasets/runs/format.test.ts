import { describe, expect, it } from "vitest";

import { costPerCase, deltaLabel, gateTone, groupRuns, passedLabel } from "./format";
import type { EvalRun, Summary } from "./types";

const summary: Summary = {
  total: 4,
  passed: 3,
  failed: 1,
  errored: 0,
  pass_rate: 0.75,
  cost_usd: 0.2,
  baseline_run_id: "run-main",
  baseline_reason: null,
  pass_rate_delta: -0.25,
  regressions: [],
  fixed: [],
};

const run = (id: string, overrides: Partial<EvalRun> = {}): EvalRun => ({
  id,
  eval: "refunds",
  agent: "refund-agent",
  dataset_id: "ds-1",
  dataset_revision: 2,
  branch: "main",
  commit_sha: "abcdef1234",
  pr_url: null,
  status: "finished",
  created_at: "2026-10-01T10:00:00Z",
  finished_at: "2026-10-01T10:05:00Z",
  summary,
  gate: { passed: true, reasons: [] },
  ...overrides,
});

describe("run formatting", () => {
  it.each([
    { delta: -0.25, expected: "−25.0 pts" },
    { delta: 0.125, expected: "+12.5 pts" },
    { delta: 0, expected: "No change" },
    { delta: null, expected: "No baseline" },
  ])("labels a delta of $delta as $expected", ({ delta, expected }) => {
    expect(deltaLabel({ ...summary, pass_rate_delta: delta })).toBe(expected);
  });

  it.each([
    { cost: 0.2, total: 4, expected: "$0.05" },
    { cost: 0.02, total: 4, expected: "$0.0050" },
    { cost: null, total: 4, expected: "–" },
    { cost: 1, total: 0, expected: "–" },
  ])("divides $cost over $total cases as $expected", ({ cost, total, expected }) => {
    expect(costPerCase({ ...summary, cost_usd: cost, total })).toBe(expected);
  });

  it("shows passed over total", () => {
    expect(passedLabel(summary)).toBe("3/4");
  });

  it.each([
    { gate: { passed: true, reasons: [] }, expected: "passed" },
    { gate: { passed: false, reasons: ["1 critical regression"] }, expected: "failed" },
    { gate: null, expected: "pending" },
  ])("reads the gate as $expected", ({ gate, expected }) => {
    expect(gateTone(run("r", { gate }))).toBe(expected);
  });

  it("splits this dataset's runs into main and pull requests, newest first", () => {
    const runs = [
      run("main-old", { created_at: "2026-10-01T00:00:00Z" }),
      run("pr", { branch: "fix-refunds", created_at: "2026-10-03T00:00:00Z" }),
      run("main-new", { created_at: "2026-10-02T00:00:00Z" }),
      run("other-dataset", { dataset_id: "ds-2" }),
    ];

    const groups = groupRuns(runs, "ds-1");

    expect({ main: groups.main.map((r) => r.id), pulls: groups.pulls.map((r) => r.id) }).toEqual({
      main: ["main-new", "main-old"],
      pulls: ["pr"],
    });
  });
});
