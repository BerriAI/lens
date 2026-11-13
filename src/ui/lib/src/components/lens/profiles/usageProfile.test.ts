import { describe, expect, it } from "vitest";
import { createLensDemo } from "../data/demo/createLensDemo";
import { summarizeUsage, usageCategory, usageProfileSchema } from "./usageProfile";

describe("recorded task categories", () => {
  it.each([
    [{ task_category: " Coding ", scenario: "Workflows" }, "Coding"],
    [{ task_category: " ", scenario: "Workflows" }, "Workflows"],
    [{ scenario: "Office automation" }, "Office automation"],
    [{}, null],
  ])("uses explicit metadata and keeps missing categories unknown", (attributes, category) => {
    expect(usageCategory(attributes)).toBe(category);
  });

  it("includes unclassified runs in the denominator", () => {
    expect(
      summarizeUsage([
        { category: "Coding", runs: 10 },
        { category: "Workflows", runs: 6 },
        { category: "Unclassified", runs: 4 },
        { category: "Unused", runs: 0 },
      ]),
    ).toEqual({
      total: 20,
      classified: 16,
      categories: [
        { category: "Coding", runs: 10 },
        { category: "Workflows", runs: 6 },
        { category: "Unclassified", runs: 4 },
      ],
    });
  });

  it("accepts ClickHouse integer strings and rejects invalid counts", () => {
    expect(usageProfileSchema.parse({ data: [{ category: "Coding", runs: "10" }] }).data).toEqual([
      { category: "Coding", runs: 10 },
    ]);
    expect(usageProfileSchema.safeParse({ data: [{ category: "Coding", runs: -1 }] }).success).toBe(false);
  });

  it("scopes demo totals to the selected agent and excludes runs outside the window", async () => {
    const now = Date.now();
    const services = createLensDemo(now);
    const window = { startMs: now - 86400000, endMs: now };
    const profile = await services.profiles.usage(window, "release_agent");
    const runs = await services.traces.list(window, "release_agent");
    expect(summarizeUsage(profile).total).toBe(runs.data.length);
    expect(profile).toEqual([
      { category: "Coding", runs: 34 },
      { category: "Workflows", runs: 1 },
    ]);
    expect(await services.profiles.usage({ startMs: now, endMs: now + 86400000 }, "release_agent")).toEqual(
      [],
    );
  });
});
