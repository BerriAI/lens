import { act, cleanup, renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { TracesApi } from "../traces/api";
import type { AgentSummary } from "./agentRollup";
import { useAgentConnectionStatus } from "./useAgentConnectionStatus";

const traces = vi.hoisted(() => ({ live: true, agents: vi.fn<TracesApi["agents"]>() }));
vi.mock("../traces/api", () => ({ useTracesApi: () => traces }));
vi.mock("../data/LensServices", () => ({ useLensAccessToken: () => "test" }));

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
  traces.agents.mockReset();
  traces.live = true;
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
});

afterEach(() => {
  cleanup();
  client.clear();
  vi.useRealTimers();
});

const advance = async (milliseconds: number) =>
  act(async () => {
    await vi.advanceTimersByTimeAsync(milliseconds);
  });

function renderConnection(name = "moyai", enabled = true) {
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return renderHook(({ name, enabled }) => useAgentConnectionStatus(name, enabled), {
    wrapper,
    initialProps: { name, enabled },
  });
}

describe("live agent connection", () => {
  it("should poll every three seconds and recognize only the exact named agent", async () => {
    traces.agents.mockResolvedValueOnce([{ ...agent, name: "Moyai" }]).mockResolvedValue([agent]);
    const { result } = renderConnection();
    expect(result.current.isChecking).toBe(true);
    await advance(1);
    expect(result.current.status).toBe("waiting");
    expect(result.current.match).toBeNull();
    expect(result.current.lastChecked).toBe(START);
    expect(traces.agents).toHaveBeenCalledOnce();

    await advance(3000);
    expect(traces.agents).toHaveBeenCalledTimes(2);
    expect(result.current.status).toBe("receiving");
    expect(result.current.match).toEqual(agent);
    expect(result.current.lastChecked).toBeGreaterThan(START);
    expect(result.current.isChecking).toBe(false);
  });

  it("should show errors over prior evidence, recover, and expire receiving even when the next request stalls", async () => {
    const old = { ...agent, last_seen: new Date(START - 7 * 86_400_000).toISOString() };
    traces.agents.mockResolvedValueOnce([old]);
    const { result } = renderConnection();
    await advance(1);
    expect(result.current.status).toBe("waiting-for-new-traces");

    traces.agents.mockRejectedValueOnce(new Error("Storage unavailable"));
    await advance(3000);
    expect(result.current.status).toBe("error");
    expect(result.current.error?.message).toBe("Storage unavailable");
    const failedAt = result.current.lastChecked;

    traces.agents.mockResolvedValueOnce([{ ...old, runs: 2 }]);
    await advance(3000);
    expect(result.current.status).toBe("receiving");
    expect(result.current.error).toBeNull();
    expect(result.current.lastChecked).toBeGreaterThan(failedAt!);

    traces.agents.mockReturnValue(new Promise(() => {}));
    await advance(60_000);
    expect(result.current.status).toBe("waiting-for-new-traces");
    expect(result.current.isChecking).toBe(true);
  });

  it.each([
    { name: "moyai", enabled: false, live: true },
    { name: "   ", enabled: true, live: true },
    { name: "moyai", enabled: true, live: false },
  ])("should not poll or manually refresh when inactive: %j", async ({ name, enabled, live }) => {
    traces.live = live;
    const { result } = renderConnection(name, enabled);
    act(() => result.current.refresh());
    await advance(9000);
    expect(traces.agents).not.toHaveBeenCalled();
    expect(result.current.isChecking).toBe(false);
    expect(result.current.lastChecked).toBeNull();
  });

  it("should stop polling when disabled or unmounted and resume when enabled", async () => {
    traces.agents.mockResolvedValue([agent]);
    const { result, rerender, unmount } = renderConnection();
    await advance(1);
    expect(traces.agents).toHaveBeenCalledOnce();

    rerender({ name: "moyai", enabled: false });
    act(() => result.current.refresh());
    await advance(9000);
    expect(traces.agents).toHaveBeenCalledOnce();

    rerender({ name: "moyai", enabled: true });
    await advance(1);
    expect(traces.agents).toHaveBeenCalledTimes(2);
    unmount();
    await advance(9000);
    expect(traces.agents).toHaveBeenCalledTimes(2);
  });

  it("should establish a new baseline when the selected agent changes", async () => {
    traces.agents.mockResolvedValueOnce([]).mockResolvedValue([agent]);
    const { result, rerender } = renderConnection();
    await advance(3001);
    expect(result.current.status).toBe("receiving");

    const other = { ...agent, name: "support" };
    traces.agents.mockResolvedValue([other]);
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
    traces.agents.mockResolvedValueOnce([]).mockResolvedValue([agent]);
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

    traces.agents.mockResolvedValue([{ ...agent, runs: 2 }]);
    await advance(3000);
    expect(client.getQueryState(directory)?.isInvalidated).toBe(true);
  });
});
