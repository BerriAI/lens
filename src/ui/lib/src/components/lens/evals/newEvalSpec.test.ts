import { describe, expect, it } from "vitest";

import { buildEvalSpec, EMPTY_DRAFT } from "./newEvalSpec";

const draft = { ...EMPTY_DRAFT, name: "agent-regressions", datasetId: "dataset-1", agent: "moyai" };

describe("buildEvalSpec", () => {
  it("builds the stored spec from the selected scorers and gate", () => {
    const result = buildEvalSpec({
      ...draft,
      calledBefore: true,
      first: " run_tests ",
      then: "open_pr",
      passRate: "90",
      critical: "",
    });
    expect(result).toEqual({
      ok: true,
      name: "agent-regressions",
      spec: {
        agent: "moyai",
        dataset_id: "dataset-1",
        scorers: [{ kind: "task_completed" }, { kind: "called_before", first: "run_tests", then: "open_pr" }],
        trials: 3,
        baseline: "main",
        gate: { regressions: 0, critical: null, pass_rate: 0.9, cost_per_case: null, min: {} },
      },
    });
  });

  it.each([
    ["an invalid name", { name: "Agent Regressions" }, "Name must be"],
    ["the reserved runs name", { name: "runs" }, "Name must be"],
    ["no dataset", { datasetId: "" }, "Pick a dataset"],
    ["no scorers", { taskCompleted: false }, "at least one scorer"],
    ["a half-filled ordering check", { calledBefore: true, first: "run_tests" }, "both tools"],
    ["an empty judge question", { judge: true }, "judge needs"],
    ["too many trials", { trials: "11" }, "1 to 10"],
    ["a pass rate over 100", { passRate: "120" }, "at most 100"],
    ["a negative cost", { costPerCase: "-1" }, "non-negative"],
  ])("rejects %s", (_, change, error) => {
    const result = buildEvalSpec({ ...draft, ...change });
    expect(result.ok).toBe(false);
    expect(result.ok ? "" : result.error).toContain(error);
  });
});
