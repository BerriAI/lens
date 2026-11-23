import { describe, expect, it } from "vitest";
import { mcpConnectionGuides } from "./mcpConnection";

describe("mcpConnectionGuides", () => {
  it("should lead with the gateway address and a LiteLLM key when Lens is served through it", () => {
    const [gateway, direct] = mcpConnectionGuides("https://gateway.example.test/", null, true);

    expect(gateway.id).toBe("gateway");
    expect(gateway.url).toBe("https://gateway.example.test/lens/mcp");
    expect(gateway.claudeCode).toContain(`lens ${gateway.url}`);
    expect(gateway.claudeCode).toContain('"Authorization: Bearer $LITELLM_API_KEY"');
    expect(gateway.codex).toBe(`codex mcp add lens --url ${gateway.url} --bearer-token-env-var LITELLM_API_KEY`);
    expect(gateway.credential).toContain("no admin token");
    expect(direct.id).toBe("direct");
    expect(direct.url).toBe("https://YOUR_LENS_ADDRESS/lens/mcp");
    expect(direct.codex).toContain("--bearer-token-env-var LENS_ADMIN_TOKEN");
  });

  it("should lead with the Lens address and the admin token when Lens is opened directly", () => {
    const [direct, gateway] = mcpConnectionGuides("http://localhost:4318", null, false);

    expect(direct.id).toBe("direct");
    expect(direct.url).toBe("http://localhost:4318/lens/mcp");
    expect(direct.claudeCode).toContain('"Authorization: Bearer $LENS_ADMIN_TOKEN"');
    expect(gateway.url).toBe("https://YOUR_LITELLM_GATEWAY/lens/mcp");
    expect(gateway.codex).toContain("--bearer-token-env-var LITELLM_API_KEY");
  });

  it("should keep the gateway's root path when it serves Lens under one", () => {
    const [gateway] = mcpConnectionGuides("https://gateway.example.test", "/litellm/lens", false);

    expect(gateway.id).toBe("gateway");
    expect(gateway.url).toBe("https://gateway.example.test/litellm/lens/mcp");
  });
});
