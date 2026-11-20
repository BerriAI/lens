import { afterEach, describe, expect, it, vi } from "vitest";

describe("absoluteTime", () => {
  afterEach(() => {
    vi.unstubAllEnvs();
    vi.resetModules();
  });

  it("shows UTC when the viewer is in another time zone", async () => {
    vi.stubEnv("TZ", "America/Los_Angeles");
    vi.resetModules();
    const { absoluteTime } = await import("./SpanHoverCard");

    expect(absoluteTime(Date.UTC(2026, 10, 20, 6, 20, 22), 2000)).toBe("11/20/2026, 06:20:24 AM UTC");
  });
});
