import { describe, expect, it } from "vitest";
import { compareModelNames, filterAnalysisModels, sortAnalysisModels } from "./analysisModels";
import type { AnalysisModelInfo } from "./types";

const catalog = (names: string[]): AnalysisModelInfo[] =>
  names.map((model_group) => ({ model_group, providers: ["gateway"] }));

describe("sortAnalysisModels", () => {
  it("orders names alphabetically with natural version numbers without changing the source", () => {
    const names = ["Zulu", "analysis-10", "Beta", "analysis-9", "Analysis-2"];
    const models = catalog(names);

    expect(sortAnalysisModels(models).map((model) => model.model_group)).toEqual([
      "Analysis-2",
      "analysis-9",
      "analysis-10",
      "Beta",
      "Zulu",
    ]);
    expect(models.map((model) => model.model_group)).toEqual(names);
  });

  it.each([
    { names: ["Alpha", "alpha", "Álpha"] },
    { names: ["analysis-02", "analysis-2"] },
  ])("breaks equivalent-name ties consistently for $names", ({ names }) => {
    expect(sortAnalysisModels(catalog(names)).map((model) => model.model_group)).toEqual(names);
    expect(sortAnalysisModels(catalog(names.toReversed())).map((model) => model.model_group)).toEqual(names);
    expect(compareModelNames(names[0], names[0])).toBe(0);
  });
});

describe("filterAnalysisModels", () => {
  const models = catalog(["analysis-2", "Beta-chat", "beta-review", "Zulu"]);

  it.each(["beta", "BETA", "  BeTa  "])("matches uppercase and lowercase names for %j", (query) => {
    expect(filterAnalysisModels(models, query)).toEqual([models[1], models[2]]);
  });

  it.each(["", "   "])("keeps the full sorted catalog for an empty query %j", (query) => {
    expect(filterAnalysisModels(models, query)).toEqual(models);
  });

  it("returns no models for an unmatched query", () => {
    expect(filterAnalysisModels(models, "missing")).toEqual([]);
  });
});
