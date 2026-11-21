import type { ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { TracesApiContext, liveTracesApi } from "../api";
import { useAgentTraces } from "./useAgentTraces";

describe("useAgentTraces", () => {
  it("passes the selected agent and the internal-traces choice, and fetches again when either changes", async () => {
    const list = vi.fn().mockResolvedValue({ data: [], next_cursor: null });
    const api = { ...liveTracesApi("sk-test"), list };
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <TracesApiContext.Provider value={api}>
        <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
      </TracesApiContext.Provider>
    );
    const { rerender } = renderHook(
      ({ agent, includeInternal }) =>
        useAgentTraces({
          accessToken: "sk-test",
          range: { hours: 24, anchorMs: 0 },
          enabled: true,
          agent,
          includeInternal,
        }),
      { initialProps: { agent: "agent-a", includeInternal: false }, wrapper },
    );

    await waitFor(() => expect(list).toHaveBeenCalledTimes(1));
    expect(list.mock.calls[0][1]).toEqual({ agent: "agent-a", includeInternal: false });

    rerender({ agent: "agent-b", includeInternal: false });
    await waitFor(() => expect(list).toHaveBeenCalledTimes(2));
    expect(list.mock.calls[1][1]).toEqual({ agent: "agent-b", includeInternal: false });

    rerender({ agent: "agent-b", includeInternal: true });
    await waitFor(() => expect(list).toHaveBeenCalledTimes(3));
    expect(list.mock.calls[2][1]).toEqual({ agent: "agent-b", includeInternal: true });
  });
});
