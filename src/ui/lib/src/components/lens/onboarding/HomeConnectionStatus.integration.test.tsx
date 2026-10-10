import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../tests/test-utils";
import fixture from "../traces/__fixtures__/trace_list.json";
import { HomeConnectionStatus } from "./HomeConnectionStatus";

const key = {
  id: "key-one",
  name: "New agent key",
  created_at: "2026-10-10T00:00:00Z",
  expires_at: null,
  tenant: { api_key_hash: "a".repeat(64), team_id: "", user_id: "test" },
};
const summary = {
  ...fixture.data[0],
  trace_id: "1".repeat(32),
  trace_ref: "B".repeat(64),
  agent_names: ["connected-agent"],
};

beforeEach(() => testQueryClient.clear());

it.each([401, 403])("reports a receipt permission failure (%s) without claiming delivery", async (status) => {
  vi.stubGlobal("fetch", vi.fn(async () => Response.json({ detail: "Access required" }, { status })));
  renderWithLens(<HomeConnectionStatus keyRecord={key} startedAt={Date.now()} enabled onRestart={vi.fn()} />);
  expect(await screen.findByText(status === 401 ? "Sign in again to check delivery." : "Your account cannot check delivery. Ask your Lens administrator for access.")).toBeVisible();
  expect(screen.queryByRole("button", { name: "Open trace" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Check again" })).not.toBeInTheDocument();
});

it("checks only traces sent after setup with the selected key, and opens the exact trace", async () => {
  const user = userEvent.setup();
  const gateway = stubGateway();
  const openTrace = vi.fn();
  const startedAt = Date.now() - 1000;
  gateway.post.mockReturnValue({
    data: [{ trace_id: summary.trace_id, trace_ref: summary.trace_ref }],
  });
  gateway.get.mockReturnValue({ summary, spans: [], agents: [] });
  renderWithLens(
    <HomeConnectionStatus
      keyRecord={key}
      startedAt={startedAt}
      enabled
      onRestart={vi.fn()}
    />,
    { onboarding: { openTrace } },
  );
  expect(await screen.findByText("Trace received")).toBeVisible();
  expect(screen.getByText(/A new trace from connected-agent/)).toBeVisible();
  expect(gateway.post).toHaveBeenCalledWith(
    "/v1/traces/query",
    expect.objectContaining({
      body: {
        sql: expect.stringContaining(
          `WHERE ApiKeyHash = '${key.tenant.api_key_hash}'`,
        ),
      },
    }),
  );
  expect(gateway.post).toHaveBeenCalledWith(
    "/v1/traces/query",
    expect.objectContaining({
      body: {
        sql: expect.stringContaining(
          `min(StartTs) >= fromUnixTimestamp64Milli(${startedAt})`,
        ),
      },
    }),
  );
  expect(gateway.get).toHaveBeenCalledWith(
    `/v1/traces/${summary.trace_id}`,
    expect.objectContaining({
      query: expect.objectContaining({ trace_ref: summary.trace_ref }),
    }),
  );
  await user.click(screen.getByRole("button", { name: "Open trace" }));
  expect(openTrace).toHaveBeenCalledWith(summary);
});

it("does not claim unrelated agent activity as setup success when no key is selected", () => {
  const gateway = stubGateway();
  gateway.get.mockReturnValue({
    agents: [
      {
        name: "background-agent",
        runs: 1,
        last_seen: new Date().toISOString(),
      },
    ],
  });
  renderWithLens(
    <HomeConnectionStatus startedAt={Date.now()} enabled onRestart={vi.fn()} />,
  );
  expect(screen.getByText("Waiting for your agent’s trace")).toBeVisible();
  expect(
    screen.queryByRole("button", { name: "Open trace" }),
  ).not.toBeInTheDocument();
  expect(gateway.get).not.toHaveBeenCalled();
  expect(gateway.post).not.toHaveBeenCalled();
});

it("retains waiting state when the selected key has no new trace", async () => {
  const gateway = stubGateway();
  gateway.post.mockReturnValue({ data: [] });
  renderWithLens(
    <HomeConnectionStatus
      keyRecord={key}
      startedAt={Date.now()}
      enabled
      onRestart={vi.fn()}
    />,
  );
  await waitFor(() => expect(gateway.post).toHaveBeenCalledOnce());
  expect(screen.getByText("Waiting for your agent’s trace")).toBeVisible();
  expect(
    screen.queryByRole("button", { name: "Open trace" }),
  ).not.toBeInTheDocument();
  expect(gateway.get).not.toHaveBeenCalled();
});

it("bounds automatic waiting and allows a saved connection to be checked again", async () => {
  const user = userEvent.setup();
  const gateway = stubGateway();
  gateway.post.mockReturnValue({
    data: [{ trace_id: summary.trace_id, trace_ref: summary.trace_ref }],
  });
  gateway.get.mockReturnValue({ summary, spans: [], agents: [] });
  renderWithLens(
    <HomeConnectionStatus
      keyRecord={key}
      startedAt={Date.now() - 360_000}
      enabled
      onRestart={vi.fn()}
    />,
  );
  expect(screen.getByText("No trace received yet")).toBeVisible();
  expect(gateway.post).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Check again" }));
  expect(await screen.findByText("Trace received")).toBeVisible();
});

it("offers recovery after a failed receipt lookup without reporting success", async () => {
  const user = userEvent.setup();
  const gateway = stubGateway();
  gateway.post.mockImplementation(() => {
    throw new Error("Storage unavailable");
  });
  renderWithLens(
    <HomeConnectionStatus
      keyRecord={key}
      startedAt={Date.now()}
      enabled
      onRestart={vi.fn()}
    />,
  );
  expect(
    await screen.findByText("Could not check for your trace"),
  ).toBeVisible();
  expect(
    screen.queryByRole("button", { name: "Open trace" }),
  ).not.toBeInTheDocument();
  gateway.post.mockReturnValue({
    data: [{ trace_id: summary.trace_id, trace_ref: summary.trace_ref }],
  });
  gateway.get.mockReturnValue({ summary, spans: [], agents: [] });
  await user.click(screen.getByRole("button", { name: "Check again" }));
  expect(await screen.findByText("Trace received")).toBeVisible();
});
