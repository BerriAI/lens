import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../tests/test-utils";
import { HomeConnectionStatus } from "./HomeConnectionStatus";

beforeEach(() => {
  testQueryClient.clear();
  vi.clearAllMocks();
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

  health.mockReturnValue({ ...service, status: { ...service.status, storage_ready: true } });
  agents.mockReturnValue({ agents: [{ ...agent, runs: 2 }] });
  await user.click(screen.getByRole("button", { name: "Check connection" }));
  expect(await screen.findByText("Receiving traces")).toBeVisible();
  expect(screen.getByText("Activation unconfirmed")).toBeVisible();
  expect(screen.queryByText("Not ready")).not.toBeInTheDocument();
  expect(screen.getByText(/^Last checked /)).toBeVisible();
  await user.click(screen.getByRole("button", { name: "View traces" }));
  expect(onOpenTraces).toHaveBeenCalledWith(agent.name);
});
