import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { renderWithLens, requestPath, stubGateway } from "../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../tests/test-utils";
import { HomeConnectionStatus } from "./HomeConnectionStatus";

beforeEach(() => {
  testQueryClient.clear();
  vi.clearAllMocks();
  viewport(true);
});

afterEach(() => vi.restoreAllMocks());

function viewport(desktop: boolean) {
  vi.spyOn(window, "matchMedia").mockImplementation((query) => ({
    matches: desktop && query === "(min-width: 1024px)",
    media: query,
    onchange: null,
    addListener: vi.fn(),
    removeListener: vi.fn(),
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    dispatchEvent: vi.fn(),
  }));
}

const readyService = {
  url: "https://lens.example",
  connected: true,
  status: { storage_ready: true, credentials_ready: true },
};

it.each([
  ["name", "Choose your project and agent name to watch for its traces here."],
  ["key", "Get a tracing key, then connect your project using the setup instructions."],
] as const)("should explain the next action at the %s stage without asking for a run", async (stage, guidance) => {
  const gateway = stubGateway();
  gateway.get.mockImplementation((path) => (path === "/lens/service" ? readyService : { agents: [] }));
  renderWithLens(<HomeConnectionStatus name="support-agent" enabled stage={stage} onOpenTraces={vi.fn()} />);

  expect(await screen.findByText("Reachable")).toBeVisible();
  expect(screen.getByRole("region", { name: "Live connection" })).toBeVisible();
  expect(screen.getByText(guidance)).toBeVisible();
  expect(screen.getByText("Waiting for traces")).toBeVisible();
  expect(screen.queryByText(/Run one task/)).not.toBeInTheDocument();
  expect(screen.queryByText(/Nothing arriving/)).not.toBeInTheDocument();
});

it("should explain a storage blocker and recheck both service health and named trace progress", async () => {
  const user = userEvent.setup();
  const gateway = stubGateway();
  const onOpenTraces = vi.fn();
  const service = {
    url: "https://lens.example",
    connected: true,
    status: { storage_ready: false, credentials_ready: true },
  };
  const agent = {
    name: "support-agent",
    runs: 1,
    failed_runs: 0,
    last_seen: new Date().toISOString(),
    frameworks: [],
  };
  const health = vi.fn(() => service);
  const agents = vi.fn(() => ({ agents: [agent] }));
  gateway.get.mockImplementation((path) => (path === "/lens/service" ? health() : agents()));
  renderWithLens(<HomeConnectionStatus name={agent.name} enabled keyReady={false} onOpenTraces={onOpenTraces} />);

  expect(screen.getByRole("region", { name: "Live connection" })).toBeVisible();
  expect(await screen.findByText("Connection needs attention")).toBeVisible();
  expect(screen.getByText("Not ready")).toBeVisible();
  expect(await screen.findByRole("button", { name: "View traces" })).toBeVisible();
  expect(screen.queryByText("Receiving traces")).not.toBeInTheDocument();

  health.mockReturnValue({
    ...service,
    status: { ...service.status, storage_ready: true },
  });
  agents.mockReturnValue({ agents: [{ ...agent, runs: 2 }] });
  await user.click(screen.getByRole("button", { name: "Check connection" }));
  expect(await screen.findByText("Receiving traces")).toBeVisible();
  expect(screen.getByText("Activation unconfirmed")).toBeVisible();
  expect(screen.queryByText("Not ready")).not.toBeInTheDocument();
  expect(screen.getByText(/^Last checked /)).toBeVisible();
  await user.click(screen.getByRole("button", { name: "View traces" }));
  expect(onOpenTraces).toHaveBeenCalledWith(agent.name);
});

it("should keep existing traces available while mobile connection details are collapsed", async () => {
  viewport(false);
  const user = userEvent.setup();
  const gateway = stubGateway();
  const onOpenTraces = vi.fn();
  const agent = {
    name: "support-agent",
    runs: 2,
    failed_runs: 0,
    last_seen: new Date().toISOString(),
    frameworks: [],
  };
  gateway.get.mockImplementation((path) => (path === "/lens/service" ? readyService : { agents: [agent] }));
  renderWithLens(<HomeConnectionStatus name={agent.name} enabled stage="key" onOpenTraces={onOpenTraces} />);

  expect(await screen.findByText("Waiting for new traces")).toBeVisible();
  const toggle = screen.getByRole("button", { name: "Connection details" });
  expect(toggle).toHaveAttribute("aria-expanded", "false");
  expect(screen.queryByRole("button", { name: "Check connection" })).not.toBeInTheDocument();
  expect(screen.getByText("Lens API")).not.toBeVisible();
  expect(screen.queryByText(/Nothing arriving/)).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "View traces" }));
  expect(onOpenTraces).toHaveBeenCalledWith(agent.name);

  await user.click(toggle);
  expect(toggle).toHaveAttribute("aria-expanded", "true");
  expect(screen.getByText("Lens API")).toBeVisible();
  expect(screen.getByRole("button", { name: "Check connection" })).toBeVisible();
  await user.click(toggle);
  expect(toggle).toHaveAttribute("aria-expanded", "false");
  expect(screen.getByText("Lens API")).not.toBeVisible();
  expect(screen.getByRole("button", { name: "View traces" })).toBeVisible();
});

it.each([
  [401, "Sign in again", "Your session is no longer valid. Sign in again to check your connection."],
  [403, "Access required", "Your account cannot check this connection. Ask your Lens administrator for access."],
] as const)("should explain a %s access failure without exposing backend details", async (status, title, guidance) => {
  viewport(false);
  vi.stubGlobal(
    "fetch",
    vi.fn<typeof fetch>(async (input) =>
      requestPath(input) === "/lens/service"
        ? Response.json(readyService)
        : Response.json({ detail: "Private backend credentials" }, { status }),
    ),
  );
  renderWithLens(<HomeConnectionStatus name="support-agent" enabled stage="key" onOpenTraces={vi.fn()} />);

  expect(await screen.findByText(title)).toBeVisible();
  expect(screen.getByText(guidance)).toBeVisible();
  expect(screen.getByRole("button", { name: "Connection details" })).toHaveAttribute("aria-expanded", "false");
  expect(screen.queryByText(/Private backend credentials/)).not.toBeInTheDocument();
  expect(screen.queryByText("Receiving traces")).not.toBeInTheDocument();
  expect(screen.queryByText("Checking every 3s")).not.toBeInTheDocument();
});
