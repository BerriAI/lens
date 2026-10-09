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
  await user.click(screen.getByRole("button", { name: "Connect project" }));
  return within(await screen.findByRole("region", { name: "Project credentials" }));
}

beforeEach(() => {
  testQueryClient.clear();
  vi.clearAllMocks();
});

describe("Lens Home connection", () => {
  it("should begin with one inline form and wait for the user to name their agent", async () => {
    const gateway = serve();
    renderWithLens(home());

    expect(screen.getByRole("heading", { name: "Connect your project" })).toBeVisible();
    expect(screen.getByRole("textbox", { name: "Agent name" })).toHaveValue("");
    expect(screen.getByRole("combobox", { name: "Your project" })).toHaveTextContent("Any agent or framework");
    expect(screen.getByRole("button", { name: "Connect project" })).toBeDisabled();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    const status = within(screen.getByRole("region", { name: "Live connection" }));
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
    expect(await credentials.findByText("Your tracing key")).toBeVisible();
    expect(gateway.post).toHaveBeenCalledWith(
      "/lens/tracing/keys",
      expect.objectContaining({ body: { name: "research_agent" } }),
    );
    expect(screen.getByRole("region", { name: "Get started with Lens" })).not.toHaveTextContent(secret);
    await user.click(credentials.getByRole("button", { name: "Copy project environment" }));
    expect(copyToClipboard).toHaveBeenLastCalledWith(expect.stringContaining(secret));
    expect(copyToClipboard).toHaveBeenLastCalledWith(
      expect.stringContaining("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT='https://lens.example/v1/traces'"),
    );

    expect(screen.getByRole("img", { name: "Claude Code logo" })).toBeVisible();
    expect(screen.getByRole("img", { name: "Codex logo" })).toBeVisible();
    expect(screen.queryByRole("tab", { name: /Claude Code|Codex/ })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Copy setup instructions" }));
    const instructions = vi.mocked(copyToClipboard).mock.lastCall?.[0];
    expect(instructions).toContain(projectSetupPrompt("https://lens.example"));
    expect(instructions).toContain('The agent name to look for in Lens is "research_agent"');
    expect(instructions).toContain("<dedicated Lens tracing key>");
    expect(instructions).not.toContain(secret);
    expect(screen.getByRole("heading", { name: "Run one task in your app" })).toBeVisible();
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
    await user.click(credentials.getByRole("button", { name: "Copy project environment" }));
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
    await user.click(screen.getByRole("button", { name: "Connect project" }));
    const credentials = within(await screen.findByRole("region", { name: "Project credentials" }));
    await user.click(credentials.getByRole("button", { name: "I already have a tracing key" }));
    expect(credentials.getByRole("alert")).toHaveTextContent("Moyai requires an HTTPS endpoint ending in /v1/traces");
    expect(credentials.queryByRole("button", { name: "Copy project environment" })).not.toBeInTheDocument();
    act(() =>
      fireEvent.change(credentials.getByRole("textbox", { name: "HTTPS traces endpoint" }), {
        target: { value: "https://lens.example/v1/traces" },
      }),
    );
    await user.click(credentials.getByRole("button", { name: "Copy project environment" }));
    expect(copyToClipboard).toHaveBeenLastCalledWith(
      "LITELLM_TRACE_ENDPOINT='https://lens.example/v1/traces'\nLITELLM_TRACE_API_KEY='<your tracing key>'",
    );
    expect(screen.queryByRole("button", { name: "Copy setup instructions" })).not.toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Run one task in your app" })).toBeVisible();
    await waitFor(() => expect(gateway.get.mock.calls.some(([path]) => path === "/v1/traces/agents")).toBe(true));
    expect(gateway.post).not.toHaveBeenCalled();
  });

  it("should resume the named project from its URL without restoring the generated secret", async () => {
    const user = userEvent.setup();
    const gateway = serve();
    const onUrlUpdate = vi.fn();
    const secret = "lens-secret-never-stored-in-route";
    gateway.post.mockReturnValue({ key: secret, active: true });
    const view = renderWithLens(home(), { onUrlUpdate });
    await chooseSelectOption(user, screen.getByRole("combobox", { name: "Your project" }), "Moyai");
    await user.click(screen.getByRole("button", { name: "Connect project" }));
    await user.click(await screen.findByRole("button", { name: "Generate tracing key" }));
    expect(await screen.findByText("Your tracing key")).toBeVisible();
    const searchParams = String(onUrlUpdate.mock.lastCall?.[0].queryString);
    expect(new URLSearchParams(searchParams).get("connect_agent")).toBe("moyai");
    expect(new URLSearchParams(searchParams).get("connect_integration")).toBe("moyai");
    expect(searchParams).not.toContain(secret);
    view.unmount();

    renderWithLens(home(), { searchParams });
    expect(await screen.findByRole("region", { name: "Project credentials" })).toBeVisible();
    expect(screen.getByText("Moyai · tracing included")).toBeVisible();
    expect(screen.queryByRole("textbox", { name: "Agent name" })).not.toBeInTheDocument();
    expect(screen.queryByText("Your tracing key")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Generate tracing key" })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Change" }));
    expect(screen.getByRole("textbox", { name: "Agent name" })).toHaveValue("moyai");
    expect(screen.getByRole("combobox", { name: "Your project" })).toHaveTextContent("Moyai");
    expect(gateway.post).toHaveBeenCalledOnce();
  });
});
