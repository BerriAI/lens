import { describe, expect, it } from "vitest";

import { diagnose } from "./diagnosis";
import { runCase, step, trial } from "./testRuns";

const baseline = runCase(true, [
  trial(1, [step("read", 0), step("run_tests", 10), step("open_pr", 20)]),
]);
const failedCheck = { scorer: "called_before", passed: false };

describe("diagnose", () => {
  it("names the skipped tool and the failing scorer in one line", () => {
    const candidate = runCase(false, [
      trial(1, [step("read", 0), step("open_pr", 20)], {
        checks: [failedCheck],
      }),
      trial(2, [step("read", 0), step("open_pr", 20)], {
        checks: [failedCheck],
      }),
      trial(3, [step("read", 0), step("run_tests", 10), step("open_pr", 20)], {
        checks: [{ ...failedCheck, passed: true }],
      }),
    ]);
    expect(diagnose(candidate, baseline).headline).toBe(
      "never called run_tests, which main called · called_before failed 2/3 trials",
    );
  });

  it("reports trial errors and tool errors as details", () => {
    const candidate = runCase(false, [
      trial(1, [step("read", 0, { ok: false })], {
        error: "trace never closed",
      }),
    ]);
    const { details } = diagnose(candidate, null);
    expect(details).toEqual([
      "tool errors: read",
      "1 trial errored: trace never closed",
    ]);
  });

  it("falls back to the judge when nothing deterministic explains the failure", () => {
    const candidate = runCase(false, baseline.trials);
    expect(diagnose(candidate, baseline).headline).toBe(
      "Failed the judge scorer",
    );
  });

  it("says a fixed case passes now", () => {
    expect(
      diagnose(baseline, runCase(false, [trial(1, [step("read", 0)])]))
        .headline,
    ).toBe("Passes now · called run_tests, open_pr, which main did not");
  });
});
