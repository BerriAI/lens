import { StrictMode, type ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { NuqsTestingAdapter } from "nuqs/adapters/testing";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Workspace } from "./workspace";

vi.mock("@litellm/lens-ui", () => ({
  configureLensHttp: vi.fn(),
  LensHostProvider: ({ children }: { children: ReactNode }) => children,
  LensWorkspace: ({ readOnly }: { readOnly: boolean }) => (
    <div>{readOnly ? "Read-only workspace" : "Authenticated workspace"}</div>
  ),
}));

const destination = "/ui/traces/trace-123?tab=spans&filter=a%2Bb#span-456";

function renderWorkspace(searchParams = "") {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <StrictMode>
      <NuqsTestingAdapter searchParams={searchParams}>
        <QueryClientProvider client={client}>
          <Workspace />
        </QueryClientProvider>
      </NuqsTestingAdapter>
    </StrictMode>,
  );
  return client;
}

function mockApi(config: unknown = { enabled: true }, configStatus = 200) {
  const fetcher = vi.fn<typeof fetch>(async (input, init) => {
    const url = new URL(String(input), window.location.origin);
    if (url.pathname.endsWith("/auth/google/config")) {
      return Response.json(config, { status: configStatus });
    }
    if (url.pathname.endsWith("/auth/session")) {
      return init?.method === "POST"
        ? Response.json({ user_id: "admin", user_role: "proxy_admin" })
        : Response.json({ error: "Unauthorized" }, { status: 401 });
    }
    throw new Error(`Unexpected request: ${url}`);
  });
  vi.stubGlobal("fetch", fetcher);
  return fetcher;
}

beforeEach(() => {
  window.history.replaceState({ destination: true }, "", destination);
  vi.stubEnv("NEXT_PUBLIC_LENS_API_URL", "");
});

describe("Lens sign-in", () => {
  it.each(["", "https://api.example.test/lens"])(
    "offers SSO through API base %j with the full local destination",
    async (apiBase) => {
      vi.stubEnv("NEXT_PUBLIC_LENS_API_URL", apiBase);
      const fetcher = mockApi();
      renderWorkspace();

      const link = await screen.findByRole("link", {
        name: "Sign in with SSO",
      });
      const url = new URL(link.getAttribute("href")!, window.location.origin);
      expect(url.origin + url.pathname).toBe(
        `${apiBase || window.location.origin}/auth/google/start`,
      );
      expect(url.searchParams.get("return_to")).toBe(destination);
      expect(fetcher).toHaveBeenCalledWith(
        `${apiBase}/auth/google/config`,
        expect.objectContaining({ method: "GET", cache: "no-store" }),
      );
      expect(screen.getByLabelText("Setup token")).toBeVisible();
      expect(
        screen.getByRole("link", { name: "Explore demo data" }),
      ).toHaveAttribute("href", "?demo=true");
    },
  );

  it.each([
    [{ enabled: false }, 200],
    [{ error: "Unavailable" }, 503],
    [{ enabled: "true" }, 200],
  ])(
    "keeps token sign-in usable without enabled SSO (%j)",
    async (config, status) => {
      const fetcher = mockApi(config, status as number);
      const client = renderWorkspace();
      await screen.findByLabelText("Setup token");
      await waitFor(() =>
        expect(client.getQueryState(["lens-sso-config"])?.fetchStatus).toBe(
          "idle",
        ),
      );
      expect(
        screen.queryByRole("link", { name: "Sign in with SSO" }),
      ).not.toBeInTheDocument();
      const user = userEvent.setup();
      await user.type(screen.getByLabelText("Setup token"), "setup-secret");
      await user.click(screen.getByRole("button", { name: "Sign in" }));

      expect(await screen.findByText("Authenticated workspace")).toBeVisible();
      expect(fetcher).toHaveBeenCalledWith(
        "/auth/session",
        expect.objectContaining({
          method: "POST",
          body: JSON.stringify({ token: "setup-secret" }),
        }),
      );
      expect(
        screen.queryByRole("link", { name: "Sign in with SSO" }),
      ).not.toBeInTheDocument();
      expect(
        window.location.pathname +
          window.location.search +
          window.location.hash,
      ).toBe(destination);
    },
  );

  it("does not offer SSO until configuration has loaded", async () => {
    let resolveConfig!: (response: Response) => void;
    const configResponse = new Promise<Response>((resolve) => {
      resolveConfig = resolve;
    });
    vi.stubGlobal(
      "fetch",
      vi.fn<typeof fetch>(async (input) =>
        String(input).endsWith("/auth/google/config")
          ? configResponse
          : Response.json({ error: "Unauthorized" }, { status: 401 }),
      ),
    );
    renderWorkspace();
    expect(await screen.findByLabelText("Setup token")).toBeVisible();
    expect(
      screen.queryByRole("link", { name: "Sign in with SSO" }),
    ).not.toBeInTheDocument();

    resolveConfig(Response.json({ enabled: true }));
    expect(
      await screen.findByRole("link", { name: "Sign in with SSO" }),
    ).toBeVisible();
  });

  it("shows a safe failure message and removes only the error marker before retrying", async () => {
    window.history.replaceState(
      { destination: true },
      "",
      "/ui/traces/trace-123?tab=spans&sso_error=failed&filter=a%2Bb#span-456",
    );
    mockApi();
    renderWorkspace();

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "SSO sign-in failed. Please try again or use your setup token.",
    );
    expect(
      window.location.pathname + window.location.search + window.location.hash,
    ).toBe(destination);
    expect(window.history.state).toEqual({ destination: true });
    const link = await screen.findByRole("link", { name: "Sign in with SSO" });
    expect(
      new URL(
        link.getAttribute("href")!,
        window.location.origin,
      ).searchParams.get("return_to"),
    ).toBe(destination);
  });

  it("opens a callback session with read-only access without another sign-in", async () => {
    const fetcher = vi.fn<typeof fetch>(async () =>
      Response.json({ user_id: "google-user", user_role: "proxy_admin_viewer" }),
    );
    vi.stubGlobal("fetch", fetcher);
    renderWorkspace();
    expect(await screen.findByText("Read-only workspace")).toBeVisible();
    expect(fetcher.mock.calls.every(([url]) => url === "/auth/session")).toBe(
      true,
    );
    expect(
      window.location.pathname + window.location.search + window.location.hash,
    ).toBe(destination);
  });

  it("opens the demo without checking session or SSO configuration", () => {
    const fetcher = mockApi();
    renderWorkspace("?demo=true");
    expect(screen.getByText("Read-only workspace")).toBeVisible();
    expect(fetcher).not.toHaveBeenCalled();
  });
});

