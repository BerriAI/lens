import { screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { renderWithProviders } from "../../../../tests/test-utils";
import { ProfileMap } from "./ProfileMap";
import type { ProfilesOverview } from "./schemas";

const overview: ProfilesOverview = {
  window: { agent: "assistant", start_ms: 0, end_ms: 1 },
  coverage: { runs_listed: 2, runs_resolved: 2, truncated: false },
  totals: {
    runs: 2,
    requests: 2,
    matched_requests: 2,
    cost_observations: 2,
    spend_usd: 1,
    other_agent_requests: 0,
    shared_requests: 0,
  },
  profiles: [
    {
      profile_id: "recorded:Coding",
      name: "Coding",
      source: "recorded",
      run_count: 2,
      share: 1,
      request_count: 2,
      cost_observations: 2,
      spend_usd: 1,
      center: { x: 50, y: 50 },
    },
  ],
  points: [
    { trace_id: "same-trace", profile_id: "recorded:Coding", x: 48, y: 50, placement: "categorical" },
    { trace_id: "same-trace", profile_id: "recorded:Coding", x: 52, y: 50, placement: "categorical" },
  ],
  taxonomy: null,
};

beforeEach(() => {
  vi.stubGlobal(
    "ResizeObserver",
    class implements ResizeObserver {
      constructor(private readonly callback: ResizeObserverCallback) {}
      observe(target: Element) {
        const size = [{ inlineSize: 600, blockSize: 320 }];
        this.callback(
          [
            {
              target,
              contentRect: new DOMRect(0, 0, 600, 320),
              borderBoxSize: size,
              contentBoxSize: size,
              devicePixelContentBoxSize: size,
            },
          ],
          this,
        );
      }
      unobserve() {}
      disconnect() {}
    },
  );
});
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it("draws runs from different teams that share a trace id without colliding", async () => {
  const errors = vi.spyOn(console, "error").mockImplementation(() => {});

  renderWithProviders(
    <ProfileMap
      overview={overview}
      colors={new Map([["recorded:Coding", "var(--chart-1)"]])}
      selected={null}
      onSelect={vi.fn()}
    />,
  );

  expect(await screen.findByText("2 runs shown")).toBeVisible();
  expect(errors.mock.calls.flat().join(" ")).not.toContain("same key");
});
