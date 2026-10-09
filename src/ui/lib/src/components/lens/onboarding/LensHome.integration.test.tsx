import { screen } from "@testing-library/react";
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

beforeEach(() => {
  testQueryClient.clear();
  window.localStorage.clear();
  window.sessionStorage.clear();
});

describe("Lens Home connection", () => {
  it("copies instructions from the first screen without naming a project or creating a key", async () => {
    const user = userEvent.setup();
    const gateway = serve();
    const onUrlUpdate = vi.fn();
    renderWithLens(home(), { onUrlUpdate });

    expect(screen.getByRole("heading", { name: "Get your first trace" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Copy setup instructions" })).toBeVisible();
    expect(screen.queryByRole("textbox", { name: "Agent name" })).not.toBeInTheDocument();
    expect(screen.queryByRole("combobox", { name: "Your project" })).not.toBeInTheDocument();
    expect(screen.queryByRole("navigation", { name: "Setup progress" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Generate tracing key" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Copy setup instructions" }));
    const instructions = await navigator.clipboard.readText();
    expect(instructions).toContain("Determine the agent name from the project");
    expect(instructions).toContain("/v1/traces/receipt");
    expect(instructions).toContain("same dedicated tracing key that exported the run");
    expect(instructions).toContain('"What would you like to instrument next?"');
    expect(gateway.post).not.toHaveBeenCalled();
    expect(gateway.put).not.toHaveBeenCalled();
    expect(gateway.delete).not.toHaveBeenCalled();
    expect(onUrlUpdate).not.toHaveBeenCalled();
  });

  it("creates a key only on request and keeps it out of copied agent instructions and shared state", async () => {
    const user = userEvent.setup();
    const gateway = serve();
    const onUrlUpdate = vi.fn();
    const secret = "lens-trace-generated-secret-for-test";
    gateway.post.mockReturnValue({ key: secret, active: true });
    renderWithLens(home(), { onUrlUpdate });

    await user.click(screen.getByText("Tracing key", { exact: true }));
    await user.click(await screen.findByRole("button", { name: "Generate tracing key" }));
    expect(await screen.findByText("Your tracing key", { exact: true })).toBeVisible();
    expect(gateway.post).toHaveBeenCalledExactlyOnceWith(
      "/lens/tracing/keys",
      expect.objectContaining({ body: { name: "Agent tracing" } }),
    );
    expect(screen.getByRole("region", { name: "Get started with Lens" })).not.toHaveTextContent(secret);
    await user.click(screen.getByRole("button", { name: "Copy setup instructions" }));
    const instructions = await navigator.clipboard.readText();
    expect(instructions).toContain("<dedicated Lens tracing key>");
    expect(instructions).toContain("Tracing key section on Lens Home");
    expect(instructions).not.toContain(secret);
    expect(JSON.stringify(onUrlUpdate.mock.calls)).not.toContain(secret);
    expect(JSON.stringify({ ...window.localStorage, ...window.sessionStorage })).not.toContain(secret);

    await user.click(screen.getByText("Set up manually", { exact: true }));
    await user.click(screen.getByRole("button", { name: "Copy project environment" }));
    const environment = await navigator.clipboard.readText();
    expect(environment).toContain(`LITELLM_TRACING_KEY='${secret}'`);
    expect(environment).toContain("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT='https://lens.example/v1/traces'");
    expect(gateway.post).toHaveBeenCalledOnce();
  });

  it.each([
    { readOnly: true, canMintTracingKey: true },
    { readOnly: false, canMintTracingKey: false },
  ])("lets restricted users copy setup instructions and use an existing key (%j)", async (onboarding) => {
    const user = userEvent.setup();
    const gateway = serve();
    renderWithLens(home(), { onboarding });

    await user.click(screen.getByRole("button", { name: "Copy setup instructions" }));
    expect(await navigator.clipboard.readText()).toContain("Load the dedicated key from this project's local environment");
    await user.click(screen.getByText("Tracing key", { exact: true }));
    expect(screen.getByText("Ask your Lens administrator for a tracing key.")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Generate tracing key" })).not.toBeInTheDocument();
    await user.click(await screen.findByText("Set up manually", { exact: true }));
    await user.click(screen.getByRole("button", { name: "Copy project environment" }));
    expect(await navigator.clipboard.readText()).toContain("LITELLM_TRACING_KEY='<your tracing key>'");
    expect(gateway.post).not.toHaveBeenCalled();
  });

  it("keeps repair instructions available when the Lens API is down", async () => {
    const user = userEvent.setup();
    const gateway = serve();
    gateway.get.mockImplementation((path) => {
      if (path === "/v1/traces/agents") return { agents: [] };
      throw new Error("Lens unavailable");
    });
    renderWithLens(home());

    await user.click(screen.getByRole("button", { name: "Copy setup instructions" }));
    const instructions = await navigator.clipboard.readText();
    expect(instructions).toContain("The Lens service state has not been verified");
    expect(instructions).toContain("Discover a tracing address reachable from my agent");
    expect(instructions).toContain("Report configuration or credential gaps instead of claiming success");
    await user.click(screen.getByText("Tracing key", { exact: true }));
    expect(screen.getByText("A working Lens connection and trace storage are needed to create a tracing key.")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Generate tracing key" })).not.toBeInTheDocument();
    expect(screen.queryByText("Set up manually", { exact: true })).not.toBeInTheDocument();
    expect(gateway.post).not.toHaveBeenCalled();
  });

  it.each(["instructions", "verify"])("uses an old %s setup link as context without restoring the wizard", async (step) => {
    const user = userEvent.setup();
    const gateway = serve();
    const onUrlUpdate = vi.fn();
    renderWithLens(home(), {
      searchParams: `?connect_agent=support-agent&connect_integration=moyai&connect_step=${step}`,
      onUrlUpdate,
    });

    await user.click(screen.getByRole("button", { name: "Copy setup instructions" }));
    const instructions = await navigator.clipboard.readText();
    expect(instructions).toContain('Setup link agent name: "support-agent"');
    expect(instructions).toContain("preserve the existing agent name when the project is already instrumented");
    expect(instructions).toContain("LITELLM_TRACE_ENDPOINT and LITELLM_TRACE_API_KEY");
    expect(screen.queryByRole("navigation", { name: "Setup progress" })).not.toBeInTheDocument();
    expect(screen.queryByRole("textbox", { name: "Agent name" })).not.toBeInTheDocument();
    expect(gateway.post).not.toHaveBeenCalled();
    expect(onUrlUpdate).not.toHaveBeenCalled();
  });

  it("discards a generated key on remount while keeping instructions available", async () => {
    const user = userEvent.setup();
    const gateway = serve();
    const secret = "lens-secret-never-stored-in-route";
    gateway.post.mockReturnValue({ key: secret, active: true });
    const view = renderWithLens(home());
    await user.click(screen.getByText("Tracing key", { exact: true }));
    await user.click(await screen.findByRole("button", { name: "Generate tracing key" }));
    await screen.findByText("Your tracing key", { exact: true });
    view.unmount();

    renderWithLens(home());
    expect(screen.getByRole("button", { name: "Copy setup instructions" })).toBeVisible();
    await user.click(await screen.findByText("Set up manually", { exact: true }));
    await user.click(screen.getByRole("button", { name: "Copy project environment" }));
    const environment = await navigator.clipboard.readText();
    expect(environment).toContain("LITELLM_TRACING_KEY='<your tracing key>'");
    expect(environment).not.toContain(secret);
    expect(gateway.post).toHaveBeenCalledOnce();
  });

  it("does not copy setup instructions or create credentials while demo data is enabled", () => {
    const gateway = serve();
    renderWithLens(<LensHome agents={agents} enabled={false} onOpenAgent={vi.fn()} onOpenAgents={vi.fn()} />);

    expect(screen.getByText("Turn off demo data to connect your project.")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Copy setup instructions" })).not.toBeInTheDocument();
    expect(screen.queryByText("Tracing key", { exact: true })).not.toBeInTheDocument();
    expect(gateway.get).not.toHaveBeenCalled();
    expect(gateway.post).not.toHaveBeenCalled();
  });
});
