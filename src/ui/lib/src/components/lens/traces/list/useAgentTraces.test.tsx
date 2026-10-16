import type { ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { TracesApiContext, liveTracesApi } from "../api";
import { spanLogWindow } from "../detail/useSpanRequestLog";
import { useAgentTraces } from "./useAgentTraces";

describe("spanLogWindow", () => {
  it("looks up the request log around the span's own time, not the logs tab window", () => {
    const spanStart = Date.parse("2026-08-01T12:00:00Z");
    expect(spanLogWindow(spanStart)).toEqual({ start_date: "2026-08-01 11:30:00", end_date: "2026-08-01 12:30:00" });
  });
});

describe("useAgentTraces", () => {
  it("passes the selected agent and fetches a new query when it changes", async () => {
    const list = vi.fn().mockResolvedValue({ data: [], next_cursor: null });
    const api = { ...liveTracesApi("sk-test"), list };
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrapper = ({ children }: { children: ReactNode }) => (
      <TracesApiContext.Provider value={api}>
        <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
      </TracesApiContext.Provider>
    );
    const { rerender } = renderHook(
      ({ agent }) => useAgentTraces({ accessToken: "sk-test", range: { hours: 24, anchorMs: 0 }, enabled: true, agent }),
      { initialProps: { agent: "agent-a" }, wrapper },
    );

    await waitFor(() => expect(list).toHaveBeenCalledTimes(1));
    expect(list.mock.calls[0][1]).toBe("agent-a");

    rerender({ agent: "agent-b" });
    await waitFor(() => expect(list).toHaveBeenCalledTimes(2));
    expect(list.mock.calls[1][1]).toBe("agent-b");
  });
});
