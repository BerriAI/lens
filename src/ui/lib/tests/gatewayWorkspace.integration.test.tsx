import {
  act,
  cleanup,
  fireEvent,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { Workspace } from "../../app/app/workspace";
import { configureLensHttp } from "../src/lib/http/configure";
import { renderWithProviders, testQueryClient } from "./test-utils";

const channel = "litellm-lens:v1";
const accessToken = "dashboard-bearer-fixture";
const origin = window.location.origin;
const parent = { postMessage: vi.fn() } as unknown as Window;
const frame = document.createElement("iframe");
const network = vi.fn<typeof fetch>();
const requests: Request[] = [];

function authenticate(
  command: unknown = { type: "auth", accessToken, headerName: "Authorization" },
  source: Window = parent,
  messageOrigin = origin,
  messageChannel = channel,
) {
  act(() => {
    window.dispatchEvent(
      new MessageEvent("message", {
        source,
        origin: messageOrigin,
        data: { channel: messageChannel, command },
      }),
    );
  });
}

beforeEach(() => {
  testQueryClient.clear();
  requests.length = 0;
  vi.mocked(parent.postMessage).mockClear();
  vi.spyOn(window, "parent", "get").mockReturnValue(parent);
  frame.setAttribute("data-lens-auth", "bearer-v1");
  vi.spyOn(window, "frameElement", "get").mockReturnValue(frame);
  window.history.replaceState(
    null,
    "",
    "/gateway/lens/ui/",
  );
  window.localStorage.clear();
  window.sessionStorage.clear();
  document.cookie = "token=another-account; path=/";
  network.mockReset();
  network.mockImplementation(async (input, init) => {
    const request =
      input instanceof Request
        ? input
        : new Request(new URL(String(input), origin), init);
    requests.push(request);
    const path = new URL(request.url).pathname.replace(
      /^\/gateway\/lens/,
      "/lens",
    );
    if (path.endsWith("/auth/session"))
      return Response.json({
        user_id: "dashboard-user",
        user_role: "proxy_admin_viewer",
      });
    if (path === "/lens")
      return Response.json({ lenses: [], workers: [], tracing_enabled: true });
    if (path.endsWith("/v1/traces/agents"))
      return Response.json({ agents: [] });
    if (path.endsWith("/service"))
      return Response.json({
        configured: true,
        connected: true,
        url: "",
        status: { storage_ready: true, credentials_ready: true },
      });
    return Response.json({
      data: [],
      next_cursor: null,
      traces: true,
      requests: false,
    });
  });
  vi.stubGlobal("fetch", network);
});

afterEach(() => {
  cleanup();
  testQueryClient.clear();
  configureLensHttp({ getBaseUrl: () => "", getAuthToken: () => null });
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  vi.useRealTimers();
  window.history.replaceState(null, "", "/");
  document.cookie = "token=; path=/; max-age=0";
});

it.each(["Authorization", "X-Gateway-Key"])(
  "waits for frame-attribute auth without a query and uses %s for session and data without cookies",
  async (headerName) => {
    renderWithProviders(<Workspace />, { searchParams: "?tab=traces" });
    expect(screen.getByRole("status")).toHaveTextContent("Opening Lens");
    expect(parent.postMessage).toHaveBeenCalledExactlyOnceWith(
      { channel, type: "auth-ready" },
      origin,
    );
    expect(network).not.toHaveBeenCalled();

    authenticate({ type: "auth", accessToken, headerName });
    expect(await screen.findByRole("heading", { name: "Traces" })).toBeVisible();
    expect(parent.postMessage).toHaveBeenCalledWith(
      expect.objectContaining({
        channel,
        type: "navigation",
        navigation: expect.objectContaining({ activeTab: "traces" }),
      }),
      origin,
    );
    expect(screen.queryByRole("tab", { name: "Traces" })).not.toBeInTheDocument();
    await waitFor(() =>
      expect(
        requests.some(
          (request) =>
            new URL(request.url).pathname === "/gateway/lens/v1/traces",
        ),
      ).toBe(true),
    );
    expect(new URL(requests[0].url).pathname).toBe(
      "/gateway/lens/auth/session",
    );
    expect(requests.length).toBeGreaterThan(1);
    for (const request of requests) {
      expect(request.headers.get(headerName)).toBe(`Bearer ${accessToken}`);
      expect(request.credentials).toBe("omit");
      if (headerName !== "Authorization")
        expect(request.headers.get("Authorization")).toBeNull();
    }
    expect(document.cookie).toContain("token=another-account");
    expect(JSON.stringify(Object.entries(window.localStorage))).not.toContain(
      accessToken,
    );
    expect(JSON.stringify(Object.entries(window.sessionStorage))).not.toContain(
      accessToken,
    );
  },
);

it("waits for fresh auth after remounting at a queryless URL with the same frame attribute", async () => {
  window.history.replaceState(null, "", "/gateway/lens/ui/?tab=traces");
  const page = renderWithProviders(<Workspace />, { searchParams: "?tab=traces" });
  authenticate();
  expect(await screen.findByRole("heading", { name: "Traces" })).toBeVisible();

  page.unmount();
  testQueryClient.clear();
  network.mockClear();
  requests.length = 0;
  window.history.replaceState(null, "", "/gateway/lens/ui/");
  renderWithProviders(<Workspace />, { searchParams: "?tab=traces" });
  expect(screen.getByRole("status")).toHaveTextContent("Opening Lens");
  expect(network).not.toHaveBeenCalled();

  const remountedToken = "remounted-dashboard-bearer";
  authenticate({ type: "auth", accessToken: remountedToken, headerName: "Authorization" });
  expect(await screen.findByRole("heading", { name: "Traces" })).toBeVisible();
  expect(requests.length).toBeGreaterThan(0);
  for (const request of requests) {
    expect(request.headers.get("Authorization")).toBe(`Bearer ${remountedToken}`);
    expect(request.credentials).toBe("omit");
  }
});

it.each([
  { name: "another origin", messageOrigin: "https://other.example" },
  { name: "another frame", source: window },
  { name: "another channel", messageChannel: "other-channel" },
  {
    name: "an empty bearer",
    command: { type: "auth", accessToken: "", headerName: "Authorization" },
  },
  {
    name: "an invalid header",
    command: { type: "auth", accessToken, headerName: "Bad Header" },
  },
  { name: "a missing header", command: { type: "auth", accessToken } },
])("ignores $name without making a session request", async (input) => {
  network.mockResolvedValue(
    Response.json({ detail: "Session expired" }, { status: 401 }),
  );
  renderWithProviders(<Workspace />);
  authenticate(
    input.command,
    input.source,
    input.messageOrigin,
    input.messageChannel,
  );
  expect(network).not.toHaveBeenCalled();
  expect(screen.getByRole("status")).toHaveTextContent("Opening Lens");

  authenticate();
  expect(
    await screen.findByText("Sign in to LiteLLM again, then reload this page."),
  ).toBeVisible();
  expect(network).toHaveBeenCalledOnce();
});

it("times out without using cookies and retries the parent handshake", async () => {
  vi.useFakeTimers();
  renderWithProviders(<Workspace />, { searchParams: "?tab=traces" });
  await act(async () => vi.advanceTimersByTime(10_001));
  expect(
    screen.getByRole("heading", { name: "Lens is not available" }),
  ).toBeVisible();
  authenticate();
  expect(network).not.toHaveBeenCalled();

  vi.useRealTimers();
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(parent.postMessage).toHaveBeenCalledTimes(2);
  expect(screen.getByRole("status")).toHaveTextContent("Opening Lens");
  authenticate();
  expect(await screen.findByRole("heading", { name: "Traces" })).toBeVisible();
});

it.each([
  { location: "/ui/", sessionPath: "/auth/session" },
  { location: "/gateway/lens/ui/", sessionPath: "/gateway/lens/auth/session" },
  {
    location: "/gateway/lens/ui/?gateway-auth=bearer-v1",
    sessionPath: "/gateway/lens/auth/session",
  },
])(
  "preserves cookie authentication at $location without the frame attribute",
  async ({ location, sessionPath }) => {
    frame.removeAttribute("data-lens-auth");
    window.history.replaceState(null, "", location);
    renderWithProviders(<Workspace />);
    expect(await screen.findByRole("tab", { name: "Traces" })).toBeVisible();
    expect(parent.postMessage).not.toHaveBeenCalledWith(
      { channel, type: "auth-ready" },
      origin,
    );
    expect(new URL(requests[0].url).pathname).toBe(sessionPath);
    expect(requests[0].credentials).toBe("same-origin");
    expect(requests[0].headers.get("Authorization")).toBeNull();
  },
);
