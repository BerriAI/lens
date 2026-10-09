import { describe, expect, it } from "vitest";
import { evalRun, type RunDetails } from "./contract";
import liveRun from "../../../../../tests/fixtures/eval-run-details";

const details: RunDetails = {
  run: {
    id: "run-1",
    status: "done",
    eval: "quality",
    agent: "support",
    version: "commit-1",
    branch: "feature",
    pr: 9,
    url: "https://lens.test/ui/?eval_run=run-1",
    expected_trials: 8,
    received_trials: 8,
    failure: "",
    summary: {
      passed: 3,
      total: 4,
      pass_rate: 0.75,
      cost_per_case: 0.25,
      errors: 2,
      scores: { task_completed: 0.75 },
      baseline_run_id: "main-1",
      baseline_version: "commit-0",
      regressions: [
        {
          case_id: "case-1",
          title: "case-1",
          critical: false,
          baseline_url: "baseline",
          candidate_url: "candidate",
        },
      ],
      fixed: [],
      gate: { passed: false, reasons: ["pass rate below minimum"] },
    },
  },
  dataset_id: "dataset-1",
  dataset_revision: 3,
  created_at: "2026-10-08T01:00:00Z",
  completed_at: "2026-10-08T01:01:00Z",
  ci_url: "",
  cases: [
    {
      case_id: "case-1",
      input: "Cancel the order",
      verdict: false,
      traces: [
        { trace_id: "trace-1", trace_ref: "ref-1" },
        { trace_id: "trace-2", trace_ref: "ref-2" },
      ],
    },
  ],
  baseline: null,
};

const baseline: RunDetails["baseline"] = {
  run: {
    ...details.run,
    id: "main-1",
    summary: { ...details.run.summary!, pass_rate: 1 },
  },
  cases: [
    {
      case_id: "case-1",
      input: "Cancel the order",
      verdict: true,
      traces: [{ trace_id: "baseline-trace", trace_ref: "baseline-ref" }],
    },
  ],
};

describe("eval run contract adapter", () => {
  it("renders the persisted Rust response from a real judge evaluation", () => {
    const run = evalRun(liveRun);
    expect(run.status).toBe("finished");
    expect(run.gate?.passed).toBe(false);
    expect(run.summary).toMatchObject({
      passed: 0,
      total: 1,
      pass_rate_delta: -1,
    });
    expect(run.summary?.regressions[0].candidate.trace_ref).toBe(
      liveRun.cases[0].traces[0].trace_ref,
    );
    expect(run.summary?.regressions[0].baseline?.trace_ref).toBe(
      liveRun.baseline.cases[0].traces[0].trace_ref,
    );
  });
  it.each([
    ["running", "running"],
    ["scoring", "running"],
    ["done", "finished"],
    ["failed", "error"],
  ] as const)("maps %s to %s", (status, expected) => {
    expect(
      evalRun({ ...details, run: { ...details.run, status } }).status,
    ).toBe(expected);
  });

  it("preserves frozen metadata, gates, trial errors, cost and exact resolved trace identities", () => {
    const run = evalRun({ ...details, baseline });
    expect(run).toMatchObject({
      dataset_id: "dataset-1",
      dataset_revision: 3,
      commit_sha: "commit-1",
      pr_number: 9,
      pr_url: null,
      gate: { passed: false, reasons: ["pass rate below minimum"] },
    });
    expect(run.summary).toMatchObject({
      passed: 3,
      failed: 1,
      errored: 2,
      cost_usd: 1,
      pass_rate_delta: -0.25,
    });
    expect(run.summary?.regressions).toEqual([
      {
        case_id: "case-1",
        input: "Cancel the order",
        critical: false,
        candidate: {
          verdict: "fail",
          trace_id: "trace-1",
          trace_ref: "ref-1",
          trace_count: 2,
        },
        baseline: {
          verdict: "pass",
          trace_id: "baseline-trace",
          trace_ref: "baseline-ref",
          trace_count: 1,
        },
      },
    ]);
  });

  it("does not invent a trace for a case with no recorded evidence", () => {
    const run = evalRun({ ...details, cases: [] });
    expect(run.summary?.regressions[0].candidate).toEqual({
      verdict: "error",
      trace_id: "",
      trace_ref: "",
      trace_count: 0,
    });
    expect(run.summary?.regressions[0].baseline).toBeNull();
  });

  it("preserves a terminal failure without manufacturing a scored summary", () => {
    const run = evalRun({
      ...details,
      run: {
        ...details.run,
        status: "failed",
        summary: null,
        failure: "judge unavailable",
      },
    });
    expect(run).toMatchObject({
      status: "error",
      failure: "judge unavailable",
      summary: null,
      gate: null,
    });
  });
});
