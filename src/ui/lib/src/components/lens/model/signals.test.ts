import { describe, expect, it } from "vitest";

import type { AnalysisModelInfo } from "./types";
import { isDecisionsMode, systemOneModels } from "./signals";

describe("isDecisionsMode", () => {
  it.each([
    { mode: "decisions", expected: true },
    { mode: "evaluation", expected: true },
    { mode: "chat", expected: false },
  ])("matches the supported Signals model modes", ({ mode, expected }) => {
    expect(isDecisionsMode(mode)).toBe(expected);
  });
});

describe("systemOneModels", () => {
  it("includes decisions and evaluation models but excludes chat models", () => {
    const details: AnalysisModelInfo[] = [
      { model_group: "chat-model", providers: ["openai"], mode: "chat" },
      { model_group: "legacy-model", providers: ["typesafe"], mode: "evaluation" },
      { model_group: "decisions-model", providers: ["typesafe"], mode: "decisions" },
    ];

    expect(systemOneModels(details).map((model) => model.model_group)).toEqual([
      "decisions-model",
      "legacy-model",
    ]);
  });
});
