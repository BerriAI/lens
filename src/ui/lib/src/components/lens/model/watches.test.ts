import { describe, expect, it } from "vitest";
import { analysisDimensions, initialWatches, setAnalysisDimension, watchChecks } from "./watches";

describe("investigation dimensions", () => {
  it("keeps new checks opt-in and preserves saved enabled checks", () => {
    expect([...initialWatches(undefined)]).toEqual(["watch_unsolved", "watch_blocked", "watch_unhappy"]);
    const enabled = new Set(["watch_performance", "watch_efficiency", "watch_telemetry"]);
    const checks = watchChecks(enabled);
    expect(checks.map(({ id }) => id)).toEqual([...enabled]);
    expect([
      ...initialWatches([...checks, { id: "watch_unsolved", enabled: false, instruction: "finish tasks" }]),
    ]).toEqual([...enabled]);
    expect(watchChecks(new Set())).toEqual([]);
  });

  it.each(analysisDimensions)("preserves saved instructions when toggling $name", (watch) => {
    const other = {
      id: "custom",
      instruction: "Check my workflow",
      enabled: true,
    };
    const saved = {
      id: watch.id,
      instruction: "A customized dimension",
      enabled: true,
    };
    const disabled = setAnalysisDimension([other, saved], watch, false);
    expect(disabled).toEqual([other, { ...saved, enabled: false }]);
    expect(setAnalysisDimension(disabled, watch, true)).toEqual([other, saved]);
    expect(setAnalysisDimension([other], watch, true)).toEqual([
      other,
      { id: watch.id, instruction: watch.instruction, enabled: true },
    ]);
  });
});
