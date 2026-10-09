import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import { chooseSelectOption, testQueryClient } from "../../../../tests/test-utils";
import { copyToClipboard } from "../../../utils/dataUtils";
import type { LensAgents } from "../agents/AgentScoped";
import { LensHome } from "./LensHome";
import { projectSetupPrompt } from "./tracing/TracingSetupCard";

vi.mock("../../../utils/dataUtils", () => ({ copyToClipboard: vi.fn().mockResolvedValue(true) }));

const agents: LensAgents = {
  agent: null,
  select: vi.fn(),
  list: { agents: [], isLoading: false, error: null },
};
const home = () => <LensHome agents={agents} enabled onOpenAgent={vi.fn()} onOpenAgents={vi.fn()} />;

function serve(url = "https://lens.example") {
  const gateway = stubGateway();
  gateway.get.mockImplementation((path) =>
    path === "/v1/traces/agents"
      ? { agents: [] }
      : { url, connected: true, status: { storage_ready: true, credentials_ready: true } },
  );
  return gateway;
}

async function connectProject(user: ReturnType<typeof userEvent.setup>, name = "research_agent") {
  act(() => fireEvent.change(screen.getByRole("textbox", { name: "Agent name" }), { target: { value: name } }));
  await user.click(screen.getByRole("button", { name: "Continue", exact: true }));
  expect(screen.getByRole("heading", { name: "Connect your project", exact: true })).toHaveFocus();
  return within(await screen.findByRole("region", { name: "Project credentials" }));
}

beforeEach(() => {
  testQueryClient.clear();
  vi.clearAllMocks();
});

describe("Lens Home connection", () => {
  it("should begin with one inline form and wait for the user to name their agent", async () => {
    const user = userEvent.setup();
    const gateway = serve();
    renderWithLens(home());

    expect(screen.getByRole("heading", { name: "Get your first trace" })).toBeVisible();
    expect(screen.getByRole("textbox", { name: "Agent name" })).toHaveValue("");
    expect(screen.getByRole("textbox", { name: "Agent name" })).toHaveAccessibleDescription(
      "Already instrumented? Use the name your app sends with its traces.",
    );
    expect(screen.getByRole("combobox", { name: "Your project" })).toHaveTextContent("Any agent or framework");
    expect(screen.getByRole("button", { name: "Continue", exact: true })).toBeDisabled();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    const status = within(screen.getByRole("region", { name: "Live connection" }));
    await user.click(status.getByRole("button", { name: "Connection details" }));
    expect(status.getByText("Name your agent to begin")).toBeVisible();
    expect(await status.findByText("Reachable")).toBeVisible();
    expect(gateway.get.mock.calls.some(([path]) => path === "/v1/traces/agents")).toBe(false);
    expect(gateway.post).not.toHaveBeenCalled();
  });

  it("should create a named key and keep it out of shared coding-agent instructions and the URL", async () => {
    const user = userEvent.setup();
    const gateway = serve();
    const onUrlUpdate = vi.fn();
    const secret = "lens-trace-generated-secret-for-test";
    gateway.post.mockReturnValue({ key: secret, active: true });
    renderWithLens(home(), { onUrlUpdate });

    const credentials = await connectProject(user, "  research_agent  ");
    await user.click(credentials.getByRole("button", { name: "Generate tracing key" }));
    expect(await credentials.findByRole("heading", { name: "Your tracing key" })).toHaveFocus();
    expect(gateway.post).toHaveBeenCalledWith(
      "/lens/tracing/keys",
      expect.objectContaining({ body: { name: "research_agent" } }),
    );
    expect(screen.getByRole("region", { name: "Get started with Lens" })).not.toHaveTextContent(secret);
    expect(screen.getByRole("button", { name: "Copy project environment" })).not.toBeVisible();
    expect(screen.getByRole("img", { name: "Claude Code logo" })).toBeVisible();
    expect(screen.getByRole("img", { name: "Codex logo" })).toBeVisible();
    expect(screen.queryByRole("tab", { name: /Claude Code|Codex/ })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Copy setup instructions" }));
    const instructions = vi.mocked(copyToClipboard).mock.lastCall?.[0];
    expect(instructions).toContain(projectSetupPrompt("https://lens.example"));
    expect(instructions).toContain('The agent name to look for in Lens is "research_agent"');
    expect(instructions).toContain("<dedicated Lens tracing key>");
    expect(instructions).not.toContain(secret);
    expect(screen.getByText(/^Instructions copied\. Paste them in your project/)).toBeVisible();
    await user.click(screen.getByText("Set up manually"));
    await user.click(screen.getByRole("button", { name: "Copy project environment" }));
    expect(copyToClipboard).toHaveBeenLastCalledWith(expect.stringContaining(secret));
    expect(copyToClipboard).toHaveBeenLastCalledWith(
      expect.stringContaining("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT='https://lens.example/v1/traces'"),
    );
    await user.click(screen.getByRole("button", { name: "Connection details" }));
    expect(within(screen.getByRole("region", { name: "Live connection" })).getByText("Ready to use")).toBeVisible();
    expect(JSON.stringify(onUrlUpdate.mock.calls)).not.toContain(secret);
    expect(gateway.post).toHaveBeenCalledOnce();
  });

  it.each([
    { readOnly: true, canMintTracingKey: true },
    { readOnly: false, canMintTracingKey: false },
  ])("should let restricted users use an existing key without offering to mint one (%j)", async (onboarding) => {
    const user = userEvent.setup();
    const gateway = serve();
    renderWithLens(home(), { onboarding });

    const credentials = await connectProject(user);
    expect(credentials.queryByRole("button", { name: "Generate tracing key" })).not.toBeInTheDocument();
    expect(credentials.getByText("Ask your Lens administrator for a tracing key.")).toBeVisible();
    await user.click(credentials.getByRole("button", { name: "I already have a tracing key" }));
    expect(credentials.getByRole("heading", { name: "Use your saved key" })).toHaveFocus();
    await user.click(screen.getByText("Set up manually"));
    await user.click(screen.getByRole("button", { name: "Copy project environment" }));
    expect(copyToClipboard).toHaveBeenLastCalledWith(
      expect.stringContaining("LITELLM_TRACING_KEY='<your tracing key>'"),
    );
    expect(screen.getByRole("button", { name: "Copy setup instructions" })).toBeVisible();
    expect(gateway.post).not.toHaveBeenCalled();
  });

  it("should use Moyai's native tracing variables after an existing key and valid HTTPS endpoint are chosen", async () => {
    const user = userEvent.setup();
    const gateway = serve("http://localhost:3100");
    renderWithLens(home());
    await chooseSelectOption(user, screen.getByRole("combobox", { name: "Your project" }), "Moyai");
    expect(screen.getByRole("textbox", { name: "Agent name" })).toHaveValue("moyai");
    await user.click(screen.getByRole("button", { name: "Continue", exact: true }));
    const credentials = within(await screen.findByRole("region", { name: "Project credentials" }));
    await user.click(credentials.getByRole("button", { name: "I already have a tracing key" }));
    const moyai = within(screen.getByRole("region", { name: "Configure Moyai" }));
    const endpoint = moyai.getByRole("textbox", { name: "HTTPS traces endpoint" });
    expect(moyai.getByRole("alert")).toHaveTextContent("Moyai requires an HTTPS endpoint ending in /v1/traces");
    expect(endpoint).toHaveAttribute("aria-invalid", "true");
    expect(endpoint).toHaveAccessibleDescription(/Moyai requires an HTTPS endpoint/);
    expect(moyai.getByRole("link", { name: "HTTPS deployment guide" })).toHaveAttribute(
      "href",
      "https://github.com/BerriAI/lens/blob/main/deploy/lens/README.md",
    );
    expect(moyai.queryByRole("button", { name: "Copy project environment" })).not.toBeInTheDocument();
    const status = within(screen.getByRole("region", { name: "Live connection" }));
    expect(await status.findByText("Connection needs attention")).toBeVisible();
    expect(status.getByText("Set an HTTPS traces endpoint in Configure Moyai before running a task.")).toBeVisible();
    act(() =>
      fireEvent.change(endpoint, {
        target: { value: "https://lens.example/v1/traces" },
      }),
    );
    expect(endpoint).toHaveAttribute("aria-invalid", "false");
    expect(moyai.queryByRole("alert")).not.toBeInTheDocument();
    expect(status.getByText("Waiting for traces")).toBeVisible();
    await user.click(moyai.getByRole("button", { name: "Copy project environment" }));
    expect(copyToClipboard).toHaveBeenLastCalledWith(
      "LITELLM_TRACE_ENDPOINT='https://lens.example/v1/traces'\nLITELLM_TRACE_API_KEY='<your tracing key>'",
    );
    expect(screen.queryByRole("button", { name: "Copy setup instructions" })).not.toBeInTheDocument();
    expect(screen.getByText(/^Restart Moyai, then run one task/)).toBeVisible();
    await waitFor(() => expect(gateway.get.mock.calls.some(([path]) => path === "/v1/traces/agents")).toBe(true));
    expect(gateway.post).not.toHaveBeenCalled();
  });

  it("should resume setup instructions without restoring the generated secret and let users request a new key", async () => {
    const user = userEvent.setup();
    const gateway = serve();
    const onUrlUpdate = vi.fn();
    const secret = "lens-secret-never-stored-in-route";
    gateway.post.mockReturnValue({ key: secret, active: true });
    const view = renderWithLens(home(), { onUrlUpdate });
    const credentials = await connectProject(user);
    await user.click(credentials.getByRole("button", { name: "Generate tracing key" }));
    expect(await credentials.findByRole("heading", { name: "Your tracing key" })).toHaveFocus();
    await waitFor(() =>
      expect(new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString).get("connect_step")).toBe("instructions"),
    );
    const searchParams = String(onUrlUpdate.mock.lastCall?.[0].queryString);
    expect(new URLSearchParams(searchParams).get("connect_agent")).toBe("research_agent");
    expect(searchParams).not.toContain(secret);
    view.unmount();

    renderWithLens(home(), { searchParams });
    expect(await screen.findByRole("region", { name: "Project credentials" })).toBeVisible();
    expect(screen.queryByRole("textbox", { name: "Agent name" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Copy setup instructions" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "Generate tracing key" })).not.toBeInTheDocument();
    await user.click(screen.getByText("Set up manually"));
    await user.click(screen.getByRole("button", { name: "Copy project environment" }));
    expect(copyToClipboard).toHaveBeenLastCalledWith(
      expect.stringContaining("LITELLM_TRACING_KEY='<your tracing key>'"),
    );
    expect(vi.mocked(copyToClipboard).mock.lastCall?.[0]).not.toContain(secret);
    await user.click(screen.getByRole("button", { name: "Need a new tracing key?" }));
    expect(screen.getByRole("heading", { name: "Get a tracing key" })).toHaveFocus();
    expect(screen.getByRole("button", { name: "Generate tracing key" })).toBeVisible();
    expect(screen.queryByRole("button", { name: "Copy setup instructions" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Change" }));
    expect(screen.getByRole("textbox", { name: "Agent name" })).toHaveValue("research_agent");
    expect(screen.getByRole("textbox", { name: "Agent name" })).toHaveFocus();
    expect(screen.getByRole("combobox", { name: "Your project" })).toHaveTextContent("Any agent or framework");
    expect(gateway.post).toHaveBeenCalledOnce();
  });

  it("should restore existing-key progress and the public Moyai endpoint after remounting", async () => {
    const user = userEvent.setup();
    const gateway = serve("http://localhost:3100");
    const onUrlUpdate = vi.fn();
    const endpoint = "https://lens.example/observe/v1/traces";
    const view = renderWithLens(home(), { onUrlUpdate });
    await chooseSelectOption(user, screen.getByRole("combobox", { name: "Your project" }), "Moyai");
    const credentials = await connectProject(user, "moyai");
    await user.click(credentials.getByRole("button", { name: "I already have a tracing key" }));
    expect(credentials.getByRole("heading", { name: "Use your saved key" })).toHaveFocus();
    act(() =>
      fireEvent.change(screen.getByRole("textbox", { name: "HTTPS traces endpoint" }), { target: { value: endpoint } }),
    );
    await waitFor(() =>
      expect(new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString).get("connect_endpoint")).toBe(endpoint),
    );
    const searchParams = String(onUrlUpdate.mock.lastCall?.[0].queryString);
    expect(new URLSearchParams(searchParams).get("connect_step")).toBe("instructions");
    view.unmount();

    renderWithLens(home(), { searchParams });
    const moyai = within(await screen.findByRole("region", { name: "Configure Moyai" }));
    expect(moyai.getByRole("textbox", { name: "HTTPS traces endpoint" })).toHaveValue(endpoint);
    expect(screen.queryByRole("button", { name: "I already have a tracing key" })).not.toBeInTheDocument();
    await user.click(moyai.getByRole("button", { name: "Copy project environment" }));
    expect(copyToClipboard).toHaveBeenLastCalledWith(
      `LITELLM_TRACE_ENDPOINT='${endpoint}'\nLITELLM_TRACE_API_KEY='<your tracing key>'`,
    );
    expect(gateway.post).not.toHaveBeenCalled();
  });

  it("should not advance the current project when an earlier project's key request finishes", async () => {
    const user = userEvent.setup();
    const gateway = serve();
    const onUrlUpdate = vi.fn();
    const pending = Promise.withResolvers<{ key: string; active: boolean }>();
    gateway.post.mockReturnValue(pending.promise);
    renderWithLens(home(), { onUrlUpdate });
    const first = await connectProject(user, "first-agent");
    await user.click(first.getByRole("button", { name: "Generate tracing key" }));
    await waitFor(() => expect(gateway.post).toHaveBeenCalledOnce());

    await user.click(screen.getByRole("button", { name: "Change" }));
    const current = await connectProject(user, "current-agent");
    await act(async () => pending.resolve({ key: "lens-private-key-for-first-agent", active: true }));

    expect(current.getByRole("heading", { name: "Get a tracing key" })).toBeVisible();
    expect(current.getByRole("button", { name: "Generate tracing key" })).toBeEnabled();
    expect(screen.queryByRole("button", { name: "Copy setup instructions" })).not.toBeInTheDocument();
    const params = new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString);
    expect(params.get("connect_agent")).toBe("current-agent");
    expect(params.has("connect_step")).toBe(false);
    expect(JSON.stringify(onUrlUpdate.mock.calls)).not.toContain("lens-private-key-for-first-agent");
    expect(gateway.post).toHaveBeenCalledOnce();
  });

  it("should guide all three steps, preserve the key when revisiting instructions, and resume verification", async () => {
    const user = userEvent.setup();
    const gateway = serve();
    const onUrlUpdate = vi.fn();
    const secret = "lens-trace-local-only-test-key";
    gateway.post.mockReturnValue({ key: secret, active: true });
    const view = renderWithLens(home(), { onUrlUpdate });
    expect(screen.getByRole("button", { name: "Step 1: Name agent" })).toHaveAttribute("aria-current", "step");
    expect(screen.getByRole("button", { name: "Step 3: Verify trace" })).toBeDisabled();
    const credentials = await connectProject(user);
    expect(screen.getByRole("button", { name: "Step 2: Connect project" })).toHaveAttribute("aria-current", "step");
    expect(screen.queryByRole("button", { name: "Continue to verification" })).not.toBeInTheDocument();
    await user.click(credentials.getByRole("button", { name: "Generate tracing key" }));
    await user.click(await screen.findByRole("button", { name: "Continue to verification" }));
    expect(screen.getByRole("button", { name: "Step 3: Verify trace" })).toHaveAttribute("aria-current", "step");
    expect(screen.getByRole("heading", { name: "Run one task" })).toHaveFocus();
    expect(screen.queryByRole("button", { name: "Copy setup instructions" })).not.toBeInTheDocument();
    expect(screen.getByText("Waiting for traces")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Step 2: Connect project" }));
    expect(await screen.findByRole("button", { name: "Copy setup instructions" })).toBeVisible();
    await user.click(screen.getByText("Set up manually"));
    await user.click(screen.getByRole("button", { name: "Copy project environment" }));
    expect(copyToClipboard).toHaveBeenLastCalledWith(expect.stringContaining(secret));
    await user.click(screen.getByRole("button", { name: "Continue to verification" }));
    await waitFor(() =>
      expect(new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString).get("connect_step")).toBe("verify"),
    );
    const searchParams = String(onUrlUpdate.mock.lastCall?.[0].queryString);
    expect(searchParams).not.toContain(secret);
    view.unmount();
    renderWithLens(home(), { searchParams });
    expect(await screen.findByRole("heading", { name: "Run one task" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Step 3: Verify trace" })).toHaveAttribute("aria-current", "step");
    expect(gateway.post).toHaveBeenCalledOnce();
  });

  it.each([
    "https://user:private-password@lens.example/v1/traces",
    "https://lens.example/v1/traces?token=private-token",
    "https://lens.example/v1/traces#private-token",
  ])("should reject and avoid persisting a sensitive endpoint (%s)", async (unsafeEndpoint) => {
    const user = userEvent.setup();
    serve();
    const onUrlUpdate = vi.fn();
    renderWithLens(home(), {
      searchParams: {
        connect_agent: "moyai",
        connect_integration: "moyai",
        connect_step: "instructions",
        connect_endpoint: "https://lens.example/v1/traces",
      },
      onUrlUpdate,
    });
    const endpoint = await screen.findByRole("textbox", { name: "HTTPS traces endpoint" });
    act(() => fireEvent.change(endpoint, { target: { value: unsafeEndpoint } }));

    expect(endpoint).toHaveAttribute("aria-invalid", "true");
    expect(screen.getByRole("alert")).toHaveTextContent("Moyai requires an HTTPS endpoint");
    expect(screen.queryByRole("button", { name: "Copy project environment" })).not.toBeInTheDocument();
    await waitFor(() => expect(onUrlUpdate).toHaveBeenCalledOnce());
    expect(new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString).has("connect_endpoint")).toBe(false);
    expect(JSON.stringify(onUrlUpdate.mock.calls)).not.toContain("private-");
    await user.click(screen.getByRole("button", { name: "Change" }));
    expect(screen.getByRole("textbox", { name: "Agent name" })).toHaveFocus();
  });
});
