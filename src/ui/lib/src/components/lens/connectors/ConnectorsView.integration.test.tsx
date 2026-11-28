import { act, fireEvent, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import {
  renderWithProviders,
  testQueryClient,
} from "../../../../tests/test-utils";
import { readRequest } from "../../../../tests/lens-test-utils";
import { LensHostProvider } from "../../../host/LensHost";
import { LensServicesProvider, liveLensServices } from "../data/LensServices";
import type { McpConnection, McpOverview } from "../../../lib/http/worker";
import { ConnectorsView } from "./ConnectorsView";

const network = vi.fn<typeof fetch>();
const now = Date.now();
const connection: McpConnection = {
  id: "test-connection",
  name: "My coding agent",
  client: "Codex",
  owner_id: "test-user",
  owner_name: "Test user",
  authentication: "token",
  created_at: new Date(now).toISOString(),
  expires_at: new Date(now + 86400000).toISOString(),
  revoked_at: null,
};
const empty: McpOverview = {
  connections: [],
  activity: { calls: [], usage: [], usage_truncated: false, next_cursor: null },
  authentication: "token",
  can_create: true,
  can_manage_all: true,
  scope: "All workspace traces",
  gateway_url: null,
  from_ms: now - 86400000,
  to_ms: now,
  retention_days: 30,
};

beforeEach(() => {
  testQueryClient.clear();
  vi.stubGlobal("fetch", network);
  network.mockReset();
});

function renderConnectors(
  surface: "standalone" | "embedded" = "standalone",
  params = "?connector=mcp",
  demo = false,
) {
  return renderWithProviders(
    <LensHostProvider host={{ surface }}>
      <LensServicesProvider services={liveLensServices("test-access")}>
        <ConnectorsView demo={demo} onExitDemo={vi.fn()} />
      </LensServicesProvider>
    </LensHostProvider>,
    { searchParams: params },
  );
}

it.each(["standalone", "embedded"] as const)(
  "creates and verifies a connection in %s",
  async (surface) => {
    const user = userEvent.setup();
    const auth = surface === "embedded" ? "gateway" : "token";
    const item = { ...connection, authentication: auth } as const;
    let data: McpOverview = { ...empty, authentication: auth };
    let posted: unknown;
    network.mockImplementation(async (input, init) => {
      const request = await readRequest(input, init);
      if (request.method === "POST") {
        posted = request.body;
        data = { ...data, connections: [item] };
        return Response.json({
          connection: item,
          token: auth === "token" ? "test-only-mcp-token" : null,
        });
      }
      return Response.json(data);
    });
    renderConnectors(surface);
    fireEvent.change(await screen.findByLabelText("Connection name"), {
      target: { value: "My coding agent" },
    });
    await user.click(screen.getByRole("button", { name: "Create connection" }));
    expect(
      await screen.findByText("My coding agent: Waiting for a tool call"),
    ).toBeVisible();
    expect(posted).toEqual(
      expect.objectContaining({
        name: "My coding agent",
        client: "Codex",
        expires_in_days: 90,
      }),
    );
    expect(
      screen.getByRole("button", { name: "Copy MCP configuration" }),
    ).toBeVisible();
    expect(screen.getByText(/codex mcp add lens/)).not.toHaveTextContent(
      "test-only-mcp-token",
    );
    data = {
      ...data,
      activity: {
        ...data.activity,
        usage: [
          {
            connection_id: item.id,
            connection_name: item.name,
            owner_id: item.owner_id,
            owner_name: item.owner_name,
            client: item.client,
            calls: 1,
            errors: 0,
            last_used_ms: now,
            last_success_ms: now,
          },
        ],
      },
    };
    await user.click(
      screen.getByRole("button", { name: "Refresh", exact: true }),
    );
    expect(await screen.findByText("My coding agent: Connected")).toBeVisible();
  },
);

it("keeps the new token available while the follow-up read is pending or fails", async () => {
  const user = userEvent.setup();
  const refresh = Promise.withResolvers<Response>();
  const readStarted = Promise.withResolvers<void>();
  network.mockImplementation(async (input, init) => {
    const request = await readRequest(input, init);
    if (request.method === "POST") {
      return Response.json({ connection, token: "test-only-mcp-token" });
    }
    if (request.query.has("connection")) {
      readStarted.resolve();
      return refresh.promise;
    }
    return Response.json(empty);
  });
  renderConnectors();
  await user.click(
    await screen.findByRole("button", { name: "Create connection" }),
  );
  await act(async () => readStarted.promise);
  expect(await screen.findByLabelText("New MCP token")).toHaveValue(
    "test-only-mcp-token",
  );
  expect(
    screen.getByText("My coding agent: Waiting for a tool call"),
  ).toBeVisible();
  await act(async () => {
    refresh.resolve(Response.json({ detail: "unavailable" }, { status: 503 }));
  });
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Could not load connections",
  );
  expect(screen.getByLabelText("New MCP token")).toHaveValue(
    "test-only-mcp-token",
  );
  await user.click(screen.getByRole("button", { name: "Copy token" }));
  expect(await navigator.clipboard.readText()).toBe("test-only-mcp-token");
  expect(
    JSON.stringify(
      testQueryClient.getQueriesData({ queryKey: ["lens-connectors"] }),
    ),
  ).not.toContain("test-only-mcp-token");
  await user.click(screen.getByRole("button", { name: "New connection" }));
  expect(await screen.findByLabelText("Connection name")).toBeVisible();
  expect(screen.queryByLabelText("New MCP token")).not.toBeInTheDocument();
});

it("revokes a connection and updates Access from the server", async () => {
  const user = userEvent.setup();
  let item = connection;
  network.mockImplementation(async (input, init) => {
    if ((await readRequest(input, init)).method === "DELETE") {
      item = { ...item, revoked_at: new Date().toISOString() };
      return Response.json(true);
    }
    return Response.json({ ...empty, connections: [item] });
  });
  renderConnectors("standalone", "?connector=mcp&mcp_section=access");
  await user.click(
    await screen.findByRole("button", { name: "Revoke", exact: true }),
  );
  const dialog = await screen.findByRole("dialog", {
    name: 'Revoke "My coding agent"?',
  });
  expect(dialog).not.toHaveTextContent("Gateway keys");
  await user.click(
    within(dialog).getByRole("button", { name: "Revoke connection" }),
  );
  expect(
    await screen.findByRole("cell", { name: "Revoked", exact: true }),
  ).toBeVisible();
  expect(network).toHaveBeenCalledWith(
    expect.stringContaining(`/lens/connectors/mcp/${connection.id}`),
    expect.objectContaining({ method: "DELETE" }),
  );
});

it("shows permission guidance and recovers from a failed read", async () => {
  const user = userEvent.setup();
  network.mockResolvedValueOnce(
    Response.json({ detail: "unavailable" }, { status: 503 }),
  );
  network.mockResolvedValue(Response.json({ ...empty, can_create: false }));
  renderConnectors();
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Could not load connections",
  );
  await user.click(
    screen.getByRole("button", { name: "Refresh", exact: true }),
  );
  expect(
    await screen.findByText(/Your role can view connections/),
  ).toBeVisible();
  expect(
    screen.queryByRole("button", { name: "Create connection" }),
  ).not.toBeInTheDocument();
});

it("does not access live credentials in demo mode", () => {
  renderConnectors("embedded", "?connector=mcp", true);
  expect(
    screen.getByRole("button", { name: "Use my deployment" }),
  ).toBeVisible();
  expect(network).not.toHaveBeenCalled();
});
