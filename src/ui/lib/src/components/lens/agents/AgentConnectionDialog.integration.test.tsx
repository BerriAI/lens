import { act, fireEvent, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import { chooseSelectOption, testQueryClient } from "../../../../tests/test-utils";
import { copyToClipboard } from "../../../utils/dataUtils";
import { AgentConnectionDialog } from "./AgentConnectionDialog";

vi.mock("../../../utils/dataUtils", () => ({ copyToClipboard: vi.fn().mockResolvedValue(true) }));

const SECRET = "lens-trace-test-secret-only";
const service = {
  url: "http://localhost:14318",
  connected: true,
  status: { storage_ready: true, credentials_ready: true },
};

const enterName = (value: string) =>
  act(() => fireEvent.change(screen.getByRole("textbox", { name: "Agent name" }), { target: { value } }));

beforeEach(() => {
  testQueryClient.clear();
  vi.clearAllMocks();
});

describe("Agent connection", () => {
  it("should explain registration and create a named key without exposing it on screen", async () => {
    const user = userEvent.setup();
    const gateway = stubGateway();
    gateway.get.mockImplementation((path) => (path === "/lens/service" ? service : { agents: [] }));
    gateway.post.mockReturnValue({ key: SECRET, active: true });
    renderWithLens(<AgentConnectionDialog open onOpenChange={vi.fn()} onConnected={vi.fn()} />);

    expect(screen.getByRole("dialog", { name: "Add an agent" })).toBeVisible();
    expect(screen.getByText("Your agent will appear in Agents when its first trace arrives")).toBeVisible();
    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
    enterName("  Moyai support  ");
    await user.click(screen.getByRole("button", { name: "Continue" }));
    await user.click(await screen.findByRole("button", { name: "Generate tracing key" }));
    await screen.findByText("Your tracing key");
    expect(gateway.post).toHaveBeenCalledWith(
      "/lens/tracing/keys",
      expect.objectContaining({ body: { name: "Moyai support" } }),
    );
    expect(screen.getByRole("dialog")).not.toHaveTextContent(SECRET);
    await user.click(screen.getByRole("button", { name: "Copy agent configuration" }));
    expect(copyToClipboard).toHaveBeenLastCalledWith(expect.stringContaining(SECRET));
    expect(copyToClipboard).toHaveBeenLastCalledWith(expect.stringContaining("http://localhost:14318/v1/traces"));
    await user.click(screen.getByText("Instrumentation example for OpenTelemetry"));
    expect(screen.getByText(/^import os/)).toHaveTextContent('AGENT_NAME = "Moyai support"');
  });

  it("should wait for the exact named agent before opening its traces", async () => {
    const user = userEvent.setup();
    const gateway = stubGateway();
    const onConnected = vi.fn();
    gateway.get.mockImplementation((path) =>
      path === "/lens/service"
        ? service
        : {
            agents: [
              { name: "other-agent", runs: 8, failed_runs: 0, last_seen: new Date().toISOString(), frameworks: [] },
            ],
          },
    );
    renderWithLens(<AgentConnectionDialog open onOpenChange={vi.fn()} onConnected={onConnected} />);
    enterName("moyai");
    await user.click(screen.getByRole("button", { name: "Continue" }));
    await user.click(await screen.findByRole("button", { name: "Check for traces" }));
    expect(await screen.findByText(/No matching traces yet/)).toBeVisible();
    expect(screen.queryByRole("button", { name: "View traces" })).not.toBeInTheDocument();
    expect(onConnected).not.toHaveBeenCalled();

    gateway.get.mockImplementation((path) =>
      path === "/lens/service"
        ? service
        : {
            agents: [{ name: "moyai", runs: 1, failed_runs: 0, last_seen: new Date().toISOString(), frameworks: [] }],
          },
    );
    await user.click(screen.getByRole("button", { name: "Check for traces" }));
    await user.click(await screen.findByRole("button", { name: "View traces" }));
    expect(onConnected).toHaveBeenCalledExactlyOnceWith("moyai");
  });

  it("should use Moyai’s native environment and require its supported HTTPS endpoint", async () => {
    const user = userEvent.setup();
    const gateway = stubGateway();
    gateway.get.mockReturnValue(service);
    renderWithLens(<AgentConnectionDialog open onOpenChange={vi.fn()} onConnected={vi.fn()} />);
    await chooseSelectOption(user, screen.getByRole("combobox", { name: "Integration" }), "Moyai");
    expect(screen.getByRole("textbox", { name: "Agent name" })).toHaveValue("moyai");
    await user.click(screen.getByRole("button", { name: "Continue" }));
    expect(await screen.findByText(/Moyai requires an HTTPS endpoint/)).toBeVisible();
    expect(screen.queryByRole("button", { name: "Copy agent configuration" })).not.toBeInTheDocument();
    act(() =>
      fireEvent.change(screen.getByRole("textbox", { name: "HTTPS traces endpoint" }), {
        target: { value: "https://lens.example/v1/traces" },
      }),
    );
    await user.click(screen.getByRole("button", { name: "Copy agent configuration" }));
    expect(copyToClipboard).toHaveBeenLastCalledWith(
      "LITELLM_TRACE_ENDPOINT='https://lens.example/v1/traces'\nLITELLM_TRACE_API_KEY='<your tracing key>'",
    );
    expect(screen.getByText(/No tracing SDK installation is needed/)).toBeVisible();
    expect(screen.queryByText(/Instrumentation example/)).not.toBeInTheDocument();
  });

  it("offers GitHub setup after the named agent’s first trace arrives", async () => {
    const user = userEvent.setup();
    const gateway = stubGateway();
    const onConnectGitHub = vi.fn();
    gateway.get.mockImplementation((path) =>
      path === "/lens/service"
        ? service
        : { agents: [{ name: "qa-agent", runs: 1, failed_runs: 0, last_seen: new Date().toISOString(), frameworks: [] }] },
    );
    renderWithLens(
      <AgentConnectionDialog
        open
        onOpenChange={vi.fn()}
        onConnected={vi.fn()}
        onConnectGitHub={onConnectGitHub}
      />,
    );
    enterName("qa-agent");
    await user.click(screen.getByRole("button", { name: "Continue" }));
    await user.click(await screen.findByRole("button", { name: "Check for traces" }));
    await user.click(await screen.findByRole("button", { name: "Connect GitHub" }));
    expect(onConnectGitHub).toHaveBeenCalledExactlyOnceWith("qa-agent");
  });

  it("should show instructions to viewers without allowing key creation", async () => {
    const user = userEvent.setup();
    const gateway = stubGateway();
    gateway.get.mockReturnValue(service);
    renderWithLens(<AgentConnectionDialog open onOpenChange={vi.fn()} onConnected={vi.fn()} />, {
      onboarding: { readOnly: true, canMintTracingKey: false },
    });
    enterName("moyai");
    await user.click(screen.getByRole("button", { name: "Continue" }));
    expect(await screen.findByText("Ask your Lens admin for a dedicated tracing key")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Generate tracing key" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Copy agent configuration" })).toBeVisible();
    expect(gateway.post).not.toHaveBeenCalled();
  });

  it("should discard the one-time tracing key when the dialog closes", async () => {
    const user = userEvent.setup();
    const gateway = stubGateway();
    gateway.get.mockReturnValue(service);
    gateway.post.mockReturnValue({ key: SECRET, active: true });
    const onOpenChange = vi.fn();
    const onConnected = vi.fn();
    const view = renderWithLens(<AgentConnectionDialog open onOpenChange={onOpenChange} onConnected={onConnected} />);
    enterName("first-agent");
    await user.click(screen.getByRole("button", { name: "Continue" }));
    await user.click(await screen.findByRole("button", { name: "Generate tracing key" }));
    await screen.findByText("Your tracing key");

    view.rerender(<AgentConnectionDialog open={false} onOpenChange={onOpenChange} onConnected={onConnected} />);
    view.rerender(<AgentConnectionDialog open onOpenChange={onOpenChange} onConnected={onConnected} />);
    expect(screen.getByRole("textbox", { name: "Agent name" })).toHaveValue("");
    enterName("second-agent");
    await user.click(screen.getByRole("button", { name: "Continue" }));
    expect(await screen.findByRole("button", { name: "Generate tracing key" })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Copy agent configuration" }));
    expect(copyToClipboard).toHaveBeenLastCalledWith(expect.stringContaining("<your tracing key>"));
  });
});
