import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../tests/test-utils";
import type { LensAgents } from "../agents/AgentScoped";
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

function serve() {
  const gateway = stubGateway();
  const records = vi.fn(() => [] as (typeof key)[]);
  gateway.get.mockImplementation((path) =>
    path === "/lens/tracing/keys"
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
    if (path === "/lens/tracing/keys") {
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
  it("offers visible key creation and complete setup instructions without requiring a name", async () => {
    const user = userEvent.setup();
    const gateway = serve();
    renderWithLens(home());
    expect(
      screen.getByRole("heading", { name: "Connect a new agent" }),
    ).toBeVisible();
    expect(
      await screen.findByRole("button", { name: "Create tracing key" }),
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
      screen.getByRole("button", { name: "Copy setup instructions" }),
    );
    expect(await navigator.clipboard.readText()).toContain(
      "/v1/traces/receipt",
    );
    expect(gateway.post).not.toHaveBeenCalled();
  });

  it("copies the new key only through the explicit action and keeps it masked and out of shared state", async () => {
    const user = userEvent.setup();
    const gateway = serve();
    const onUrlUpdate = vi.fn();
    renderWithLens(home(), { onUrlUpdate });
    await user.click(
      await screen.findByRole("button", { name: "Create tracing key" }),
    );
    expect(
      await screen.findByText("Your tracing key", { exact: true }),
    ).toBeVisible();
    expect(gateway.post).toHaveBeenCalledExactlyOnceWith(
      "/lens/tracing/keys",
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
    expect(instructions).toContain("ignored local environment file");
    expect(instructions).toContain("/v1/traces/receipt");
    await user.click(screen.getByRole("button", { name: "Manage tracing keys" }));
    await user.click(await screen.findByRole("button", { name: "Use key in my project" }));
    expect(screen.getByRole("button", { name: "Copy instructions with key" })).toBeVisible();
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
      "Load the dedicated key from this project's local environment",
    );
    await user.click(screen.getByRole("button", { name: "Set up manually" }));
    await user.click(
      screen.getByRole("button", { name: "Copy project environment" }),
    );
    expect(await navigator.clipboard.readText()).toContain(
      `LITELLM_TRACING_KEY='${secret}'`,
    );
    expect(
      gateway.post.mock.calls.filter(([path]) => path === "/lens/tracing/keys"),
    ).toHaveLength(1);
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
      renderWithLens(home(), { onboarding });
      expect(
        screen.getByText(/Ask your Lens administrator for a tracing key/),
      ).toBeVisible();
      expect(
        screen.queryByRole("button", { name: "Create tracing key" }),
      ).not.toBeInTheDocument();
      await user.click(
        screen.getByRole("button", { name: "My project already has one" }),
      );
      await user.click(
        screen.getByRole("button", { name: "Copy setup instructions" }),
      );
      expect(await navigator.clipboard.readText()).toContain(
        "Load the dedicated key from this project's local environment",
      );
      expect(gateway.post).not.toHaveBeenCalled();
      await user.click(screen.getByRole("button", { name: "Manage tracing keys" }));
      expect(screen.queryByRole("button", { name: "Revoke", exact: true })).not.toBeInTheDocument();
      await user.click(await screen.findByRole("button", { name: "Use key in my project" }));
      expect(screen.getByRole("button", { name: "Choose another key" })).toBeVisible();
      expect(screen.queryByRole("button", { name: "Create a new key" })).not.toBeInTheDocument();
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
      path === "/lens/tracing/keys" ? [] : service(),
    );
    renderWithLens(home());
    expect(await screen.findByText(/Trace storage is not ready/)).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Create tracing key" }),
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
      await screen.findByRole("button", { name: "Create tracing key" }),
    ).toBeVisible();
    expect(
      screen.queryByText(/Trace storage is not ready/),
    ).not.toBeInTheDocument();
  });

  it("resumes the selected connection on refresh without persisting or regenerating its secret", async () => {
    const user = userEvent.setup();
    const gateway = serve();
    const view = renderWithLens(home());
    await user.click(
      await screen.findByRole("button", { name: "Create tracing key" }),
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
    renderWithLens(home());
    expect(await screen.findByText("Using Agent tracing")).toBeVisible();
    expect(screen.getByText(/Lens cannot show this key again/)).toBeVisible();
    expect(screen.getByText("Waiting for your agent’s trace")).toBeVisible();
    await user.click(
      screen.getByRole("button", { name: "Copy setup instructions" }),
    );
    expect(await navigator.clipboard.readText()).not.toContain(secret);
    expect(
      gateway.post.mock.calls.filter(([path]) => path === "/lens/tracing/keys"),
    ).toHaveLength(1);
  });

  it("selects an existing key and revokes it only after confirming the affected agent connection", async () => {
    const user = userEvent.setup();
    const gateway = serve();
    gateway.records.mockReturnValue([key]);
    renderWithLens(home());
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
        "/lens/tracing/keys/key-one",
        expect.anything(),
      ),
    );
    expect(await screen.findByText(/No tracing keys yet/)).toBeVisible();
    expect(screen.queryByText("Using Agent tracing")).not.toBeInTheDocument();
  });

  it("preserves legacy setup context without treating it as a received trace", async () => {
    const user = userEvent.setup();
    serve();
    renderWithLens(home(), {
      searchParams: "?connect_agent=support-agent&connect_step=verify",
    });
    await user.click(
      screen.getByRole("button", { name: "Copy setup instructions" }),
    );
    expect(await navigator.clipboard.readText()).toContain(
      'Setup link agent name: "support-agent"',
    );
    expect(
      screen.queryByText("Trace received", { exact: true }),
    ).not.toBeInTheDocument();
  });

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
});
