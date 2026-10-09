import { describe, expect, it } from "vitest";

import { gateLabel, scorerLabel } from "./labels";

describe("eval labels", () => {
  it("describes ordering scorers by the tools they compare", () => {
    expect(scorerLabel({ kind: "called_before", first: "run_tests", then: "open_pr" })).toBe("run_tests before open_pr");
  });

  it("lists only the gate conditions that are set", () => {
    expect(gateLabel({ regressions: 0, critical: null, pass_rate: 0.9 })).toBe("regressions ≤ 0 · pass ≥ 90%");
    expect(gateLabel({})).toBe("no gate");
  });
});
