import { describe, expect, it } from "vitest";
import { SYSTEM_ONE_MODE, systemOneModels } from "./signals";
import type { AnalysisModelInfo } from "./types";

describe("systemOneModels", () => {
  it("keeps decisions and legacy evaluation models and drops chat models", () => {
    const models: AnalysisModelInfo[] = [
      { model_group: "canonical", providers: [], mode: "decisions" },
      { model_group: "legacy", providers: [], mode: "evaluation" },
      { model_group: "chat", providers: [], mode: "chat" },
    ];

    expect(SYSTEM_ONE_MODE).toBe("decisions");
    expect(systemOneModels(models).map((model) => model.model_group)).toEqual([
      "canonical",
      "legacy",
    ]);
  });
});
