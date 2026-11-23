import { describe, expect, it } from "vitest";
import { mcpConnectionGuide } from "./mcpConnection";

describe("mcpConnectionGuide", () => {
  it("should point both agents at the gateway with a LiteLLM key when Lens is served through it", () => {
    const guide = mcpConnectionGuide("https://gateway.example.test/", true);

    expect(guide.url).toBe("https://gateway.example.test/lens/mcp");
    expect(guide.claudeCode).toContain(`lens ${guide.url}`);
    expect(guide.claudeCode).toContain('"Authorization: Bearer $LITELLM_API_KEY"');
    expect(guide.codex).toBe(`codex mcp add lens --url ${guide.url} --bearer-token-env-var LITELLM_API_KEY`);
    expect(guide.credential).toContain("LiteLLM key");
  });

  it("should use the Lens admin token when Lens is opened directly", () => {
    const guide = mcpConnectionGuide("http://localhost:4318", false);

    expect(guide.url).toBe("http://localhost:4318/lens/mcp");
    expect(guide.claudeCode).toContain('"Authorization: Bearer $LENS_ADMIN_TOKEN"');
    expect(guide.codex).toContain("--bearer-token-env-var LENS_ADMIN_TOKEN");
    expect(guide.credential).toContain("LENS_ADMIN_TOKEN");
  });
});
