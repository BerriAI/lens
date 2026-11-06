import { expect, it } from "vitest";
import { traceTagSetSchema, traceTagsParser } from "./tags";

it("preserves exact recorded metadata values in tag responses and shared filters", () => {
  const tag = { key: "scenario", value: "Coding " };
  const response = traceTagSetSchema.parse({ recorded: [tag], manual: [], editable: false });
  expect(response.recorded).toEqual([tag]);
  expect(traceTagsParser.parse(JSON.stringify(response.recorded))).toEqual([tag]);
});
