import { describe, expect, it } from "vitest";

import {
  costPerCase,
  deltaLabel,
  deltaTone,
  gateTone,
  groupRuns,
  passedLabel,
  shortSha,
} from "./format";
import { diff, evalRun, summary } from "./testRuns";

describe("run formatting", () => {
  it.each([
    {
      name: "no baseline",
      value: summary({ baseline_run_id: null }),
      expected: "No baseline",
    },
    { name: "no change", value: summary(), expected: "No change" },
    {
      name: "regressed",
      value: summary({ regressions: [diff("a"), diff("b")] }),
      expected: "2 regressed",
    },
    {
      name: "both",
      value: summary({ regressions: [diff("a")], fixed: [diff("b")] }),
      expected: "1 regressed · 1 fixed",
    },
  ])("labels $name as $expected", ({ value, expected }) => {
    expect(deltaLabel(value)).toBe(expected);
  });

  it.each([
    {
      name: "regressions",
      value: summary({ regressions: [diff("a")], fixed: [diff("b")] }),
      expected: "text-destructive",
    },
    {
      name: "only fixes",
      value: summary({ fixed: [diff("b")] }),
      expected: "text-success",
    },
    {
      name: "no baseline",
      value: summary({ baseline_run_id: null, regressions: [diff("a")] }),
      expected: "text-muted-foreground",
    },
    { name: "no summary", value: null, expected: "text-muted-foreground" },
  ])("tones $name as $expected", ({ value, expected }) => {
    expect(deltaTone(value)).toBe(expected);
  });

  it.each([
    { each: 0.05, total: 4, expected: "$0.05" },
    { each: 0.005, total: 4, expected: "$0.0050" },
    { each: 0, total: 4, expected: "$0.00" },
    { each: 1, total: 0, expected: "–" },
  ])(
    "shows $each per case over $total cases as $expected",
    ({ each, total, expected }) => {
      expect(costPerCase(summary({ cost_per_case: each, total }))).toBe(
        expected,
      );
    },
  );

  it("shows passed over total and a 7 character sha", () => {
    expect(passedLabel(summary())).toBe("3/4");
    expect(shortSha("abcdef1234")).toBe("abcdef1");
  });

  it.each([
    { name: "passed", value: evalRun("r"), expected: "passed" },
    {
      name: "failed",
      value: evalRun("r", {
        summary: summary({ gate: { passed: false, reasons: ["x"] } }),
      }),
      expected: "failed",
    },
    {
      name: "pending",
      value: evalRun("r", { status: "scoring", summary: null }),
      expected: "pending",
    },
    {
      name: "errored",
      value: evalRun("r", { status: "failed", summary: null }),
      expected: "errored",
    },
  ])("reads the gate as $expected for a $name run", ({ value, expected }) => {
    expect(gateTone(value)).toBe(expected);
  });

  it("splits runs into main and pull requests keeping server order", () => {
    const runs = [
      evalRun("pr-2", { pr: 2 }),
      evalRun("main-1"),
      evalRun("pr-1", { pr: 1 }),
    ];
    const { main, pulls } = groupRuns(runs);
    expect(main.map((run) => run.id)).toEqual(["main-1"]);
    expect(pulls.map((run) => run.id)).toEqual(["pr-2", "pr-1"]);
  });
});
