import { describe, expect, it } from "vitest";
import type { McpConnection, McpUsage } from "../../../lib/http/worker";
import {
  AGENT_INSTRUCTIONS,
  CLIENTS,
  connectionEndpoint,
  connectionStatus,
  installSnippet,
} from "./model";

const connection: McpConnection = {
  id: "connection",
  name: "Agent",
  client: "Codex",
  authentication: "token",
  owner_id: "admin",
  owner_name: "Owner label",
  created_at: "2026-01-01T00:00:00Z",
  expires_at: "2026-04-01T00:00:00Z",
  revoked_at: null,
};
const usage: McpUsage = {
  connection_id: "connection",
  connection_name: "Agent",
  owner_id: "admin",
  owner_name: "Owner label",
  client: "Codex",
  calls: 1,
  errors: 0,
  last_used_ms: 1,
  last_success_ms: 1,
};

describe("connection state", () => {
  it.each([
    ["waiting", connection, undefined, "Waiting for a tool call"],
    ["successful call", connection, usage, "Connected"],
    [
      "failed call",
      connection,
      { ...usage, errors: 1, last_success_ms: null },
      "Needs attention",
    ],
    [
      "revoked after success",
      { ...connection, revoked_at: connection.created_at },
      usage,
      "Revoked",
    ],
    [
      "expired after success",
      { ...connection, expires_at: connection.created_at },
      usage,
      "Expired",
    ],
  ] as const)("handles %s", (_, item, activity, expected) => {
    expect(
      connectionStatus(item, activity, Date.parse("2026-02-01T00:00:00Z")),
    ).toBe(expected);
  });
});

it.each([
  [
    "standalone",
    { authentication: "token", embedded: false, pathname: "/ui/", base: "" },
    "https://lens.example/lens/mcp/connection",
  ],
  [
    "embedded with prefix",
    {
      authentication: "gateway",
      embedded: true,
      pathname: "/ui/",
      base: "https://gateway.example/prefix",
    },
    "https://gateway.example/prefix/lens/mcp/connection",
  ],
  [
    "gateway hosted",
    {
      authentication: "gateway",
      embedded: false,
      pathname: "/prefix/lens/ui/",
      base: "",
    },
    "https://lens.example/prefix/lens/mcp/connection",
  ],
  [
    "standalone gateway login",
    {
      authentication: "gateway",
      embedded: false,
      pathname: "/ui/",
      base: "",
      gatewayUrl: "https://gateway.example/prefix",
    },
    "https://gateway.example/prefix/lens/mcp/connection",
  ],
  [
    "standalone token on gateway",
    { authentication: "token", embedded: true, pathname: "/ui/", base: "" },
    null,
  ],
  [
    "missing gateway",
    { authentication: "gateway", embedded: false, pathname: "/ui/", base: "" },
    null,
  ],
  [
    "credentials in URL",
    {
      authentication: "token",
      embedded: false,
      pathname: "/ui/",
      base: "https://user:password@lens.example",
    },
    null,
  ],
] as const)(
  "resolves %s without asking for deployment mode",
  (_, input, expected) => {
    expect(
      connectionEndpoint({
        ...input,
        origin: "https://lens.example",
        id: "connection",
      }),
    ).toBe(expected);
  },
);

it.each(CLIENTS)(
  "generates %s setup with an environment reference",
  (client) => {
    const snippet = installSnippet(
      client,
      "https://lens.example/lens/mcp/id",
      "token",
    );
    expect(snippet).toContain("LENS_MCP_TOKEN");
    expect(snippet).not.toContain("LENS_ADMIN_TOKEN");
  },
);

it("quotes shell URLs and preserves Cursor's environment expression", () => {
  expect(
    installSnippet("Codex", "https://lens.example/a'b", "gateway"),
  ).toContain("'https://lens.example/a'\\''b'");
  expect(
    JSON.parse(
      installSnippet("Cursor", "https://lens.example/lens/mcp/id", "token"),
    ).mcpServers.lens.headers.Authorization,
  ).toBe("Bearer ${env:LENS_MCP_TOKEN}");
  expect(AGENT_INSTRUCTIONS).toContain("unknown cost is not zero");
});
