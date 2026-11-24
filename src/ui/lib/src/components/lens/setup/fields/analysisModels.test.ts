import { expect, it } from "vitest";
import { analysisModelOptions } from "./analysisModels";

it("lists unique Findings models in the same natural alphabetical order as the model browser", () => {
  const names = ["Zulu", "analysis-10", "Beta", "beta", "analysis-9", "Analysis-2", "Beta"];
  const options = analysisModelOptions(names, [
    { model_group: "Beta", providers: ["gateway"], mode: "chat", supported_openai_params: ["response_format"] },
  ]);

  expect(options.map(({ value }) => value)).toEqual([
    "Analysis-2",
    "analysis-9",
    "analysis-10",
    "Beta",
    "beta",
    "Zulu",
  ]);
  expect(options.find(({ value }) => value === "Beta")).toEqual({
    value: "Beta",
    label: "Beta",
    sublabel: "gateway · JSON output supported",
  });
});
