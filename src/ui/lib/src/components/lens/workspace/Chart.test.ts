import { describe, expect, it } from "vitest";
import { chartSpec } from "./Chart";

describe("chartSpec", () => {
  it("preserves evidence while letting Lens own size and theme", () => {
    const data = { values: [{ outcome: "failed", count: 3 }] };
    const spec = chartSpec(
      JSON.stringify({
        data,
        mark: "bar",
        width: 2000,
        height: 1000,
        background: "black",
        title: "Poster",
        config: { font: "serif" },
      }),
      true,
    );
    expect(spec.data).toEqual(data);
    expect(spec.width).toBe("container");
    expect(spec.height).toBeLessThan(300);
    expect(spec).not.toHaveProperty("config");
    expect(spec).not.toHaveProperty("background");
    expect(spec).not.toHaveProperty("title");
  });
  it.each([
    { data: { url: "https://example.com/data" } },
    { mark: "point", encoding: { href: { field: "target" } } },
  ])("rejects remote resources and links: %j", (spec) => {
    expect(() => chartSpec(JSON.stringify(spec), false)).toThrow(
      "Charts must contain their data",
    );
  });
});
