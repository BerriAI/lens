import { act, cleanup, renderHook } from "@testing-library/react";
import { focusManager, onlineManager, QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LensServicesProvider, liveLensServices } from "../data/LensServices";
import { createLensDemo } from "../data/demo/createLensDemo";
import { requestPath } from "../../../../tests/lens-test-utils";
import { ApiError } from "../../../lib/http/client";
import type { AgentSummary } from "./agentRollup";
import { useAgentConnectionStatus } from "./useAgentConnectionStatus";

const agents = vi.fn<() => Promise<readonly AgentSummary[]>>();

const START = Date.parse("2026-10-09T12:00:00Z");
const agent: AgentSummary = {
  name: "moyai",
  runs: 1,
  failed_runs: 0,
  frameworks: [],
  last_seen: new Date(START).toISOString(),
};

let client: QueryClient;

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(START);
  agents.mockReset();
  vi.stubGlobal(
    "fetch",
    vi.fn<typeof fetch>(async (input) => {
      expect(requestPath(input)).toBe("/v1/traces/agents");
      try {
        return Response.json({ agents: await agents() });
      } catch (error) {
        if (error instanceof ApiError) return Response.json({ detail: error.message }, { status: error.status });
        throw error;
      }
    }),
  );
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
});

afterEach(() => {
  cleanup();
  client.clear();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

const advance = async (milliseconds: number) =>
  act(async () => {
    await vi.advanceTimersByTimeAsync(milliseconds);
  });

function renderConnection(name = "moyai", enabled = true, discoverAfter?: number, live = true) {
  const services = live ? liveLensServices("test") : createLensDemo(START);
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>
      <LensServicesProvider services={services}>{children}</LensServicesProvider>
    </QueryClientProvider>
  );
  return renderHook(({ name, enabled }) => useAgentConnectionStatus(name, enabled, discoverAfter), {
    wrapper,
    initialProps: { name, enabled },
  });
}

describe("live agent connection", () => {
  it("should discover a named agent only after a newly started run and keep polling after a failed check", async () => {
    const old = { ...agent, last_seen: new Date(START - 60_000).toISOString() };
    agents.mockResolvedValue([old]);
    const { result } = renderConnection("", true, START);
    await advance(1);
    expect(result.current.status).toBe("waiting");
    expect(result.current.match).toBeNull();
    const [input] = vi.mocked(fetch).mock.calls[0];
    const url = new URL(input instanceof Request ? input.url : String(input), "http://localhost");
    expect(url.searchParams.get("start_ms")).toBe(String(START));

    agents.mockRejectedValueOnce(new Error("Offline"));
    await advance(3000);
    expect(result.current.status).toBe("error");

    agents.mockResolvedValue([{ ...old, runs: 2 }]);
    await advance(3000);
    expect(result.current.status).toBe("waiting");
    expect(result.current.match).toBeNull();

    const fresh = {
      ...agent,
      runs: 3,
      last_seen: new Date(START + 7000).toISOString(),
    };
    agents.mockResolvedValue([fresh]);
    await advance(3000);
    expect(result.current.status).toBe("receiving");
    expect(result.current.match).toEqual(fresh);
    expect(result.current.error).toBeNull();
  });

  it("should discover a trace arriving before the first request without mistaking demo data for an arrival", async () => {
    agents.mockResolvedValue([agent]);
    const live = renderConnection("", true, START);
    await advance(1);
    expect(live.result.current.status).toBe("receiving");
    live.unmount();

    const demo = renderConnection("", true, START, false);
    await advance(9000);
    expect(demo.result.current.status).toBe("waiting");
    expect(demo.result.current.match).toBeNull();
    expect(agents).toHaveBeenCalledOnce();
  });

  it("should poll every three seconds and recognize only the exact named agent", async () => {
    agents.mockResolvedValueOnce([{ ...agent, name: "Moyai" }]).mockResolvedValue([agent]);
    const { result } = renderConnection();
    expect(result.current.isChecking).toBe(true);
    await advance(1);
    expect(result.current.status).toBe("waiting");
    expect(result.current.match).toBeNull();
    expect(result.current.lastChecked).toBe(START);
    expect(agents).toHaveBeenCalledOnce();

    await advance(3000);
    expect(agents).toHaveBeenCalledTimes(2);
    expect(result.current.status).toBe("receiving");
    expect(result.current.match).toEqual(agent);
    expect(result.current.lastChecked).toBeGreaterThan(START);
    expect(result.current.isChecking).toBe(false);
  });

  it("should show errors over prior evidence, recover, and expire receiving even when the next request stalls", async () => {
    const old = {
      ...agent,
      last_seen: new Date(START - 7 * 86_400_000).toISOString(),
    };
    agents.mockResolvedValueOnce([old]);
    const { result } = renderConnection();
    await advance(1);
    expect(result.current.status).toBe("waiting-for-new-traces");

    agents.mockRejectedValueOnce(new Error("Storage unavailable"));
    await advance(3000);
    expect(result.current.status).toBe("error");
    expect(result.current.error?.message).toBe("Storage unavailable");
    const failedAt = result.current.lastChecked;

    agents.mockResolvedValueOnce([{ ...old, runs: 2 }]);
    await advance(3000);
    expect(result.current.status).toBe("receiving");
    expect(result.current.error).toBeNull();
    expect(result.current.lastChecked).toBeGreaterThan(failedAt!);

    agents.mockReturnValue(new Promise(() => {}));
    await advance(60_000);
    expect(result.current.status).toBe("waiting-for-new-traces");
    expect(result.current.isChecking).toBe(true);
  });

  it.each([
    { name: "moyai", enabled: false, live: true },
    { name: "   ", enabled: true, live: true },
    { name: "moyai", enabled: true, live: false },
  ])("should not poll or manually refresh when inactive: %j", async ({ name, enabled, live }) => {
    const { result } = renderConnection(name, enabled, undefined, live);
    act(() => result.current.refresh());
    await advance(9000);
    expect(agents).not.toHaveBeenCalled();
    expect(result.current.isChecking).toBe(false);
    expect(result.current.lastChecked).toBeNull();
  });

  it("should stop polling when disabled or unmounted and resume when enabled", async () => {
    agents.mockResolvedValue([agent]);
    const { result, rerender, unmount } = renderConnection();
    await advance(1);
    expect(agents).toHaveBeenCalledOnce();

    rerender({ name: "moyai", enabled: false });
    act(() => result.current.refresh());
    await advance(9000);
    expect(agents).toHaveBeenCalledOnce();

    rerender({ name: "moyai", enabled: true });
    await advance(1);
    expect(agents).toHaveBeenCalledTimes(2);
    unmount();
    await advance(9000);
    expect(agents).toHaveBeenCalledTimes(2);
  });

  it("should establish a new baseline when the selected agent changes", async () => {
    agents.mockResolvedValueOnce([]).mockResolvedValue([agent]);
    const { result, rerender } = renderConnection();
    await advance(3001);
    expect(result.current.status).toBe("receiving");

    const other = { ...agent, name: "support" };
    agents.mockResolvedValue([other]);
    rerender({ name: "support", enabled: true });
    await advance(1);
    expect(result.current.match).toEqual(other);
    expect(result.current.status).toBe("waiting-for-new-traces");
  });

  it("should refresh the live agents directory only when a newly observed run changes its evidence", async () => {
    const directory = ["lensAgents", "test", true];
    const demoDirectory = ["lensAgents", "test", false];
    const otherAccount = ["lensAgents", "another-token", true];
    for (const key of [directory, demoDirectory, otherAccount]) client.setQueryData(key, []);
    agents.mockResolvedValueOnce([]).mockResolvedValue([agent]);
    renderConnection();
    await advance(1);
    expect(client.getQueryState(directory)?.isInvalidated).toBe(false);

    await advance(3000);
    expect(client.getQueryState(directory)?.isInvalidated).toBe(true);
    expect(client.getQueryState(demoDirectory)?.isInvalidated).toBe(false);
    expect(client.getQueryState(otherAccount)?.isInvalidated).toBe(false);
    client.setQueryData(directory, [agent]);
    await advance(3000);
    expect(client.getQueryState(directory)?.isInvalidated).toBe(false);

    agents.mockResolvedValue([{ ...agent, runs: 2 }]);
    await advance(3000);
    expect(client.getQueryState(directory)?.isInvalidated).toBe(true);
  });

  it("should refresh an empty directory when the first check already finds the named agent", async () => {
    const directory = ["lensAgents", "test", true];
    client.setQueryData(directory, []);
    agents.mockResolvedValue([agent]);
    const { result } = renderConnection();
    await advance(1);
    expect(result.current.status).toBe("waiting-for-new-traces");
    expect(result.current.match).toEqual(agent);
    expect(client.getQueryState(directory)?.isInvalidated).toBe(true);

    client.setQueryData(directory, [agent]);
    await advance(3000);
    expect(result.current.status).toBe("waiting-for-new-traces");
    expect(client.getQueryState(directory)?.isInvalidated).toBe(false);
  });

  it.each([
    { status: 401, name: "moyai", discoverAfter: undefined },
    { status: 403, name: "moyai", discoverAfter: undefined },
    { status: 401, name: "", discoverAfter: START },
    { status: 403, name: "", discoverAfter: START },
  ])(
    "should stop automatic checks after an access failure and resume after manual recovery: %j",
    async ({ status, name, discoverAfter }) => {
      agents.mockRejectedValue(new ApiError("Private backend detail", status, {}));
      const { result } = renderConnection(name, true, discoverAfter);
      await advance(1);
      expect(result.current.status).toBe("error");
      expect(result.current.isChecking).toBe(false);
      await advance(12_000);
      act(() => {
        focusManager.setFocused(false);
        focusManager.setFocused(true);
        onlineManager.setOnline(false);
        onlineManager.setOnline(true);
      });
      await advance(1);
      expect(agents).toHaveBeenCalledOnce();

      agents.mockResolvedValue([agent]);
      act(() => result.current.refresh());
      await advance(1);
      expect(result.current.status).toBe(discoverAfter === undefined ? "waiting-for-new-traces" : "receiving");
      expect(result.current.error).toBeNull();
      expect(agents).toHaveBeenCalledTimes(2);
      await advance(3000);
      expect(agents).toHaveBeenCalledTimes(3);
    },
  );
});
