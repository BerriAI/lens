import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../tests/test-utils";
import * as clipboard from "../../../utils/dataUtils";
import type { LensAgents } from "../agents/AgentScoped";
import { LensHostProvider } from "../../../host/LensHost";
import { LensHome } from "./LensHome";

const agents: LensAgents = {
  agent: null,
  select: vi.fn(),
  list: { agents: [], isLoading: false, error: null },
};
const home = () => <LensHome agents={agents} enabled onOpenAgents={vi.fn()} />;
const key = {
  id: "key-one",
  name: "Agent tracing",
  created_at: "2026-10-10T00:00:00Z",
  expires_at: null,
  tenant: { api_key_hash: "a".repeat(64), team_id: "", user_id: "test" },
};
const secret = "lens-trace-generated-secret-for-test";

async function standaloneHome(options?: Parameters<typeof renderWithLens>[1]) {
  const view = renderWithLens(home(), options);
  await userEvent.click(screen.getByRole("radio", { name: /Standalone Lens/ }));
  return view;
}

function serve() {
  const gateway = stubGateway();
  const records = vi.fn(() => [] as (typeof key)[]);
  gateway.get.mockImplementation((path) =>
    path === "/lens/admin/tracing/keys"
      ? records()
      : path === "/v1/traces/agents"
        ? { agents: [] }
        : {
            url: "https://lens.example",
            connected: true,
            status: { storage_ready: true, credentials_ready: true },
          },
  );
  gateway.post.mockImplementation((path) => {
    if (path === "/lens/admin/tracing/keys") {
      records.mockReturnValue([key]);
      return { key: secret, active: true, record: key };
    }
    return { data: [] };
  });
  return { ...gateway, records };
}

beforeEach(() => {
  testQueryClient.clear();
  window.localStorage.clear();
  window.sessionStorage.clear();
});

describe("Lens Home connection", () => {
  it("creates a key and copies complete setup instructions in one action without requiring a name", async () => {
    const user = userEvent.setup();
    const gateway = serve();
    await standaloneHome();
    expect(
      screen.getByRole("heading", { name: "Connect a new agent" }),
    ).toBeVisible();
    expect(
      await screen.findByRole("button", {
        name: "Copy instructions with new key",
      }),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Manage tracing keys" }),
    ).toBeVisible();
    expect(
      screen.queryByRole("textbox", { name: "Agent name" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText("Lens API", { exact: true }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText("Trace storage", { exact: true }),
    ).not.toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Copy instructions with new key" }),
    );
    expect(await navigator.clipboard.readText()).toContain(
      "CLAUDE_CODE_ENABLE_TELEMETRY=1",
    );
    expect(await navigator.clipboard.readText()).toContain(secret);
    expect(
      gateway.post.mock.calls.filter(
        ([path]) => path === "/lens/admin/tracing/keys",
      ),
    ).toHaveLength(1);
  });

  it("copies the new key only through the explicit action and keeps it masked and out of shared state", async () => {
    const user = userEvent.setup();
    const gateway = serve();
    const onUrlUpdate = vi.fn();
    await standaloneHome({ onUrlUpdate });
    await user.click(
      await screen.findByRole("button", {
        name: "Copy instructions with new key",
      }),
    );
    expect(
      await screen.findByText("Your tracing key", { exact: true }),
    ).toBeVisible();
    expect(gateway.post).toHaveBeenCalledWith(
      "/lens/admin/tracing/keys",
      expect.objectContaining({ body: { name: "Agent tracing" } }),
    );
    await user.click(screen.getByText("View full instructions"));
    expect(
      screen.getByRole("region", { name: "Connect an agent" }),
    ).not.toHaveTextContent(secret);
    expect(
      screen.getByText(/the key will be in its chat history/),
    ).toBeVisible();
    await user.click(
      screen.getByRole("button", { name: "Copy instructions with key" }),
    );
    const instructions = await navigator.clipboard.readText();
    expect(instructions).toContain(secret);
    expect(instructions).toContain("Bearer <the Lens tracing key>");
    await user.click(
      screen.getByRole("button", { name: "Manage tracing keys" }),
    );
    await user.click(
      await screen.findByRole("button", { name: "Use key in my project" }),
    );
    expect(
      screen.getByRole("button", { name: "Copy instructions with key" }),
    ).toBeVisible();
    expect(screen.getByText("Waiting for your agent’s trace")).toBeVisible();
    expect(JSON.stringify(onUrlUpdate.mock.calls)).not.toContain(secret);
    expect(
      JSON.stringify({ ...window.localStorage, ...window.sessionStorage }),
    ).not.toContain(secret);
    expect(
      JSON.stringify(
        testQueryClient
          .getQueryCache()
          .getAll()
          .map((query) => query.state.data),
      ),
    ).not.toContain(secret);
    await user.click(screen.getByRole("button", { name: "Copy without key" }));
    expect(await navigator.clipboard.readText()).not.toContain(secret);
    expect(await navigator.clipboard.readText()).toContain(
      "Use my Lens tracing key from the LITELLM_TRACING_KEY environment variable",
    );
    await user.click(screen.getByRole("button", { name: "Set up manually" }));
    await user.click(
      screen.getByRole("button", { name: "Copy project environment" }),
    );
    expect(await navigator.clipboard.readText()).toContain(
      `LITELLM_TRACING_KEY='${secret}'`,
    );
    expect(
      gateway.post.mock.calls.filter(
        ([path]) => path === "/lens/admin/tracing/keys",
      ),
    ).toHaveLength(1);
  });

  it("keeps manual key creation ahead of configuration and reuses that key for the agent prompt", async () => {
    const user = userEvent.setup();
    const gateway = serve();
    await standaloneHome();
    await user.click(screen.getByRole("button", { name: "Set up manually" }));
    await user.click(
      await screen.findByRole("button", { name: "Create tracing key" }),
    );
    await user.click(
      screen.getByRole("button", { name: "Copy project environment" }),
    );
    expect(await navigator.clipboard.readText()).toContain(secret);
    expect(await navigator.clipboard.readText()).toContain(
      "https://lens.example/v1/traces",
    );
    await user.click(
      screen.getByRole("button", { name: "Use a coding agent" }),
    );
    await user.click(
      screen.getByRole("button", { name: "Copy instructions with key" }),
    );
    expect(await navigator.clipboard.readText()).toContain(secret);
    expect(
      gateway.post.mock.calls.filter(
        ([path]) => path === "/lens/admin/tracing/keys",
      ),
    ).toHaveLength(1);
  });

  it("preserves the new key after a clipboard failure and retries without creating another", async () => {
    const user = userEvent.setup();
    const gateway = serve();
    const copy = vi
      .spyOn(clipboard, "copyToClipboard")
      .mockResolvedValueOnce(false);
    await standaloneHome();
    await user.click(
      await screen.findByRole("button", {
        name: "Copy instructions with new key",
      }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Your key is saved on this page",
    );
    expect(screen.getByText("Run one task")).toBeVisible();
    copy.mockRestore();
    await user.click(
      screen.getByRole("button", { name: "Copy instructions with key" }),
    );
    expect(await navigator.clipboard.readText()).toContain(secret);
    expect(screen.getByText("Waiting for your agent’s trace")).toBeVisible();
    expect(
      gateway.post.mock.calls.filter(
        ([path]) => path === "/lens/admin/tracing/keys",
      ),
    ).toHaveLength(1);
  });

  it("reports key creation failure and lets the user retry the same action", async () => {
    const user = userEvent.setup();
    const gateway = serve();
    gateway.post.mockRejectedValueOnce(new Error("Unavailable"));
    await standaloneHome();
    await user.click(
      await screen.findByRole("button", {
        name: "Copy instructions with new key",
      }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Could not create a tracing key",
    );
    expect(screen.getByText("Run one task")).toBeVisible();
    await user.click(
      screen.getByRole("button", { name: "Copy instructions with new key" }),
    );
    expect(await navigator.clipboard.readText()).toContain(secret);
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it.each([
    { readOnly: true, canMintTracingKey: true },
    { readOnly: false, canMintTracingKey: false },
  ])(
    "supports existing keys and an administrator handoff for restricted users (%j)",
    async (onboarding) => {
      const user = userEvent.setup();
      const gateway = serve();
      gateway.records.mockReturnValue([key]);
      await standaloneHome({ onboarding });
      expect(
        screen.getByText(/Ask your Lens administrator for a tracing key/),
      ).toBeVisible();
      expect(
        screen.queryByRole("button", {
          name: "Copy instructions with new key",
        }),
      ).not.toBeInTheDocument();
      await user.click(
        screen.getByRole("button", { name: "Use an existing key" }),
      );
      await user.click(
        screen.getByRole("button", { name: "Copy setup instructions" }),
      );
      expect(await navigator.clipboard.readText()).toContain(
        "Use my Lens tracing key from the LITELLM_TRACING_KEY environment variable",
      );
      expect(gateway.post).not.toHaveBeenCalled();
      await user.click(
        screen.getByRole("button", { name: "Manage tracing keys" }),
      );
      expect(
        screen.queryByRole("button", { name: "Revoke", exact: true }),
      ).not.toBeInTheDocument();
      await user.click(
        await screen.findByRole("button", { name: "Use key in my project" }),
      );
      expect(
        screen.getByRole("button", { name: "Choose another key" }),
      ).toBeVisible();
      expect(
        screen.queryByRole("button", { name: "Create a new key" }),
      ).not.toBeInTheDocument();
    },
  );

  it("recovers from unavailable storage without reloading or losing the setup instructions", async () => {
    const user = userEvent.setup();
    const gateway = serve();
    const service = vi.fn(() => ({
      url: "https://lens.example",
      connected: true,
      status: { storage_ready: false },
    }));
    gateway.get.mockImplementation((path) =>
      path === "/lens/admin/tracing/keys" ? [] : service(),
    );
    await standaloneHome();
    expect(await screen.findByText(/Trace storage is not ready/)).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Copy instructions with new key" }),
    ).not.toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Copy setup instructions" }),
    );
    expect(await navigator.clipboard.readText()).toContain(
      "Diagnose ClickHouse first",
    );
    service.mockReturnValue({
      url: "https://lens.example",
      connected: true,
      status: { storage_ready: true },
    });
    await user.click(screen.getByRole("button", { name: "Retry connection" }));
    expect(
      await screen.findByRole("button", {
        name: "Copy instructions with new key",
      }),
    ).toBeVisible();
    expect(
      screen.queryByText(/Trace storage is not ready/),
    ).not.toBeInTheDocument();
  });

  it("resumes the selected connection on refresh without persisting or regenerating its secret", async () => {
    const user = userEvent.setup();
    const gateway = serve();
    const view = await standaloneHome();
    await user.click(
      await screen.findByRole("button", {
        name: "Copy instructions with new key",
      }),
    );
    await user.click(
      await screen.findByRole("button", { name: "Copy instructions with key" }),
    );
    await waitFor(() =>
      expect(gateway.post).toHaveBeenCalledWith(
        "/v1/traces/query",
        expect.anything(),
      ),
    );
    view.unmount();
    await standaloneHome();
    expect(await screen.findByText("Using Agent tracing")).toBeVisible();
    expect(screen.getByText(/Lens cannot show this key again/)).toBeVisible();
    expect(screen.getByText("Waiting for your agent’s trace")).toBeVisible();
    await user.click(
      screen.getByRole("button", { name: "Copy setup instructions" }),
    );
    expect(await navigator.clipboard.readText()).not.toContain(secret);
    expect(
      gateway.post.mock.calls.filter(
        ([path]) => path === "/lens/admin/tracing/keys",
      ),
    ).toHaveLength(1);
  });

  it("selects an existing key and revokes it only after confirming the affected agent connection", async () => {
    const user = userEvent.setup();
    const gateway = serve();
    gateway.records.mockReturnValue([key]);
    await standaloneHome();
    await user.click(
      screen.getByRole("button", { name: "Manage tracing keys" }),
    );
    const dialog = within(
      await screen.findByRole("dialog", { name: "Manage tracing keys" }),
    );
    await user.click(
      await dialog.findByRole("button", { name: "Use key in my project" }),
    );
    expect(screen.getByText("Using Agent tracing")).toBeVisible();
    expect(gateway.post).not.toHaveBeenCalled();
    await user.click(
      screen.getByRole("button", { name: "Manage tracing keys" }),
    );
    await user.click(
      await screen.findByRole("button", { name: "Revoke", exact: true }),
    );
    expect(gateway.delete).not.toHaveBeenCalled();
    expect(
      screen.getByText(/Agents using this key will stop sending traces/),
    ).toBeVisible();
    gateway.records.mockReturnValue([]);
    await user.click(screen.getByRole("button", { name: "Confirm revoke" }));
    await waitFor(() =>
      expect(gateway.delete).toHaveBeenCalledWith(
        "/lens/admin/tracing/keys/key-one",
        expect.anything(),
      ),
    );
    expect(await screen.findByText(/No tracing keys yet/)).toBeVisible();
    expect(screen.queryByText("Using Agent tracing")).not.toBeInTheDocument();
  });

  it("does not treat a legacy setup link as a received trace", async () => {
    serve();
    await standaloneHome({
      searchParams: "?connect_agent=support-agent&connect_step=verify",
    });
    expect(
      await screen.findByRole("button", {
        name: "Copy instructions with new key",
      }),
    ).toBeVisible();
    expect(
      screen.queryByText("Trace received", { exact: true }),
    ).not.toBeInTheDocument();
  });

  it.each(["gateway", "standalone"] as const)(
    "copies instructions for the chosen coding agent on the %s path",
    async (path) => {
      const user = userEvent.setup();
      serve();
      if (path === "standalone") await standaloneHome();
      else renderWithLens(home());
      const agents = screen.getByRole("radiogroup", { name: "Coding agent" });
      expect(
        within(agents)
          .getAllByRole("radio")
          .map((chip) => chip.textContent),
      ).toEqual(["Claude Code", "Codex", "OpenCode"]);
      expect(
        within(agents).getByRole("radio", { name: "Claude Code" }),
      ).toBeChecked();
      await user.click(within(agents).getByRole("radio", { name: "Codex" }));
      expect(
        within(agents).getByRole("radio", { name: "Codex" }),
      ).toBeChecked();
      expect(
        screen.getByText("Paste these instructions into Codex."),
      ).toBeVisible();
      await user.click(
        screen.getByRole("button", {
          name:
            path === "gateway"
              ? "Copy setup instructions"
              : "Copy instructions with new key",
        }),
      );
      const copied = await navigator.clipboard.readText();
      expect(copied).toContain("~/.codex/config.toml");
      expect(copied).not.toContain("CLAUDE_CODE_ENABLE_TELEMETRY");
      expect(copied).not.toMatch(/README|github\.com|\/docs\//);
    },
  );

  it("does not copy instructions or create credentials while demo data is enabled", () => {
    const gateway = serve();
    renderWithLens(
      <LensHome agents={agents} enabled={false} onOpenAgents={vi.fn()} />,
    );
    expect(
      screen.getByText("Turn off demo data to connect your project."),
    ).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Copy setup instructions" }),
    ).not.toBeInTheDocument();
    expect(gateway.get).not.toHaveBeenCalled();
    expect(gateway.post).not.toHaveBeenCalled();
  });

  it("defaults to the gateway path inside the LiteLLM dashboard and copies instructions that use the LiteLLM key without creating a Lens tracing key", async () => {
    const user = userEvent.setup();
    const gateway = serve();
    renderWithLens(home());
    expect(
      screen.getByRole("radio", { name: /Through the LiteLLM gateway/ }),
    ).toBeChecked();
    expect(
      screen.getByRole("heading", {
        name: "Connect through the LiteLLM gateway",
      }),
    ).toBeVisible();
    await user.click(
      screen.getByRole("button", { name: "Copy setup instructions" }),
    );
    const copied = await navigator.clipboard.readText();
    expect(copied).toContain("the same LiteLLM key");
    expect(copied).toContain("Do not create or use a Lens tracing key");
    expect(copied).not.toContain("/v1/traces/receipt");
    expect(
      screen.queryByRole("button", { name: "Manage tracing keys" }),
    ).not.toBeInTheDocument();
    expect(
      gateway.post.mock.calls.filter(
        ([path]) => path === "/lens/admin/tracing/keys",
      ),
    ).toHaveLength(0);
  });

  it.each([
    { gateway_connected: true, checked: /Through the LiteLLM gateway/ },
    { gateway_connected: false, checked: /Standalone Lens/ },
    { gateway_connected: undefined, checked: /Standalone Lens/ },
  ])(
    "opens the standalone app on the path matching gateway_connected=$gateway_connected",
    async ({ gateway_connected, checked }) => {
      const gateway = serve();
      const get = gateway.get.getMockImplementation();
      gateway.get.mockImplementation((path, ...rest) => {
        const body = get?.(path, ...rest);
        return path === "/lens/service" ? { ...body, gateway_connected } : body;
      });
      renderWithLens(
        <LensHostProvider host={{ surface: "standalone" }}>
          {home()}
        </LensHostProvider>,
      );
      await waitFor(() =>
        expect(gateway.get).toHaveBeenCalledWith(
          "/lens/service",
          expect.anything(),
        ),
      );
      expect(await screen.findByRole("radio", { name: checked })).toBeChecked();
    },
  );

  it("opens on the gateway path when a gateway serves the app under its own mount", async () => {
    window.history.replaceState(null, "", "/litellm/lens/ui/");
    try {
      serve();
      renderWithLens(
        <LensHostProvider host={{ surface: "standalone" }}>
          {home()}
        </LensHostProvider>,
      );
      expect(
        await screen.findByRole("radio", {
          name: /Through the LiteLLM gateway/,
        }),
      ).toBeChecked();
    } finally {
      window.history.replaceState(null, "", "/");
    }
  });

  it("keeps the path a user picked in the standalone app when a connected gateway is reported", async () => {
    const user = userEvent.setup();
    const gateway = serve();
    const get = gateway.get.getMockImplementation();
    const service = vi.fn(() => ({ gateway_connected: true }));
    gateway.get.mockImplementation((path, ...rest) => {
      const body = get?.(path, ...rest);
      return path === "/lens/service" ? { ...body, ...service() } : body;
    });
    renderWithLens(
      <LensHostProvider host={{ surface: "standalone" }}>
        {home()}
      </LensHostProvider>,
    );
    expect(
      await screen.findByRole("radio", {
        name: /Through the LiteLLM gateway/,
        checked: true,
      }),
    ).toBeVisible();
    await user.click(screen.getByRole("radio", { name: /Standalone Lens/ }));
    service.mockClear();
    await testQueryClient.refetchQueries({ queryKey: ["lens-service"] });
    expect(service).toHaveBeenCalled();
    expect(
      screen.getByRole("radio", { name: /Standalone Lens/ }),
    ).toBeChecked();
  });

  it("keeps the public address and tracing credential notices off the gateway path", async () => {
    const user = userEvent.setup();
    const gateway = serve();
    const get = gateway.get.getMockImplementation();
    gateway.get.mockImplementation((path, ...rest) => {
      const body = get?.(path, ...rest);
      return path === "/lens/admin/tracing/keys" || path === "/v1/traces/agents"
        ? body
        : { ...body, url: "" };
    });
    renderWithLens(home());
    await user.click(screen.getByRole("radio", { name: /Standalone Lens/ }));
    expect(
      await screen.findByText(/Set your public Lens address/),
    ).toBeVisible();
    await user.click(
      screen.getByRole("radio", { name: /Through the LiteLLM gateway/ }),
    );
    expect(
      screen.queryByText(/Set your public Lens address/),
    ).not.toBeInTheDocument();
  });

  it("shows gateway environment for manual setup and switches to the Lens tracing key flow for standalone", async () => {
    const user = userEvent.setup();
    serve();
    renderWithLens(home());
    await user.click(screen.getByRole("button", { name: "Set up manually" }));
    expect(
      screen.getByText(/export CLAUDE_CODE_ENABLE_TELEMETRY="1"/),
    ).toBeVisible();
    expect(
      screen.getByText(
        /export OTEL_EXPORTER_OTLP_HEADERS="Authorization=Bearer <your LiteLLM key>"/,
      ),
    ).toBeVisible();
    await user.click(screen.getByRole("radio", { name: "OpenCode" }));
    expect(
      screen.getByText(/export OTEL_BSP_SCHEDULE_DELAY="1000"/),
    ).toBeVisible();
    expect(
      screen.queryByText(/CLAUDE_CODE_ENABLE_TELEMETRY/),
    ).not.toBeInTheDocument();
    expect(screen.queryByText(/LITELLM_TRACING_KEY/)).not.toBeInTheDocument();
    await user.click(screen.getByRole("radio", { name: /Standalone Lens/ }));
    expect(
      screen.getByRole("heading", { name: "Connect to Lens standalone" }),
    ).toBeVisible();
    expect(
      screen.queryByText(/OTEL_BSP_SCHEDULE_DELAY/),
    ).not.toBeInTheDocument();
    expect(
      await screen.findByRole("button", { name: "Create tracing key" }),
    ).toBeVisible();
  });
});
