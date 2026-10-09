import { act, cleanup, renderHook } from "@testing-library/react";
import { focusManager, onlineManager, QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "../../../../lib/http/client";
import { apiClient } from "../../../../lib/http/requests";
import { useLensService } from "./TracingSetupCard";

vi.mock("../../../../lib/http/requests", () => ({ apiClient: { get: vi.fn() } }));

let client: QueryClient;
const service = {
  connected: true,
  url: "https://lens.example",
  status: { storage_ready: true, credentials_ready: true },
};
const advance = async (milliseconds: number) =>
  act(async () => {
    await vi.advanceTimersByTimeAsync(milliseconds);
  });

beforeEach(() => {
  vi.useFakeTimers();
  vi.mocked(apiClient.get).mockReset();
  client = new QueryClient();
});

afterEach(() => {
  cleanup();
  client.clear();
  vi.useRealTimers();
});

function renderService(token = "first-session") {
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return renderHook(({ token }) => useLensService(token, { refetchInterval: 3000 }), {
    wrapper,
    initialProps: { token },
  });
}

describe("Lens service checks", () => {
  it.each([401, 403])("should stop automatic checks after HTTP %s and allow manual recovery", async (status) => {
    vi.mocked(apiClient.get).mockRejectedValue(new ApiError("Private backend detail", status, {}));
    const { result } = renderService();
    await advance(1);
    expect(result.current.isError).toBe(true);
    expect(result.current.isFetching).toBe(false);
    await advance(12_000);
    act(() => {
      focusManager.setFocused(false);
      focusManager.setFocused(true);
      onlineManager.setOnline(false);
      onlineManager.setOnline(true);
    });
    await advance(1);
    expect(apiClient.get).toHaveBeenCalledOnce();

    vi.mocked(apiClient.get).mockResolvedValue(service);
    act(() => void result.current.refetch());
    await advance(1);
    expect(result.current.data).toEqual(service);
    expect(result.current.error).toBeNull();
    expect(apiClient.get).toHaveBeenCalledTimes(2);
    await advance(3000);
    expect(apiClient.get).toHaveBeenCalledTimes(3);
  });

  it("should check again when a different session replaces one without access", async () => {
    vi.mocked(apiClient.get).mockRejectedValue(new ApiError("Forbidden", 403, {}));
    const { result, rerender } = renderService();
    await advance(1);
    vi.mocked(apiClient.get).mockResolvedValue(service);
    rerender({ token: "new-session" });
    await advance(1);
    expect(result.current.data).toEqual(service);
    expect(apiClient.get).toHaveBeenLastCalledWith("/lens/service", { accessToken: "new-session" });
  });
});
