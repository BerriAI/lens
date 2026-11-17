import { describe, expect, it } from "vitest";

import type { AnalysisModelInfo } from "./types";
import { chatJudgeModels, isDecisionsMode, systemOneModels } from "./signals";

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

describe("chatJudgeModels", () => {
  it("offers only chat models, in name order", () => {
    const details: AnalysisModelInfo[] = [
      { model_group: "sonnet", providers: ["anthropic"], mode: "chat" },
      { model_group: "decisions-model", providers: ["typesafe"], mode: "decisions" },
      { model_group: "embeddings", providers: ["openai"], mode: "embedding" },
      { model_group: "haiku", providers: ["anthropic"], mode: "chat" },
    ];

    expect(chatJudgeModels(details).map((model) => model.model_group)).toEqual(["haiku", "sonnet"]);
  });
});
