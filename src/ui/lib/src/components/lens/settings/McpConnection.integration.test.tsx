import { screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { renderWithLens } from "../../../../tests/lens-test-utils";
import { LensHostProvider } from "../../../host/LensHost";
import { McpConnection } from "./McpConnection";

describe("MCP connection", () => {
  it("should lead with the LiteLLM key setup when Lens runs inside the gateway", async () => {
    renderWithLens(<McpConnection />);

    const gateway = await screen.findByRole("region", { name: "Through LiteLLM" });
    expect(within(gateway).getByText(`${window.location.origin}/lens/mcp`)).toBeVisible();
    expect(within(gateway).getByText(/no admin token needed/)).toBeVisible();
    expect(within(gateway).getByRole("button", { name: "Copy Claude Code command, Through LiteLLM" })).toBeVisible();
    expect(within(gateway).getByRole("button", { name: "Copy Codex command, Through LiteLLM" })).toBeVisible();
    const direct = screen.getByRole("region", { name: "Directly to Lens" });
    expect(within(direct).getByText(/Use LENS_ADMIN_TOKEN/)).toBeVisible();
    expect(gateway.compareDocumentPosition(direct)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    expect(screen.getByRole("link", { name: "Docs" })).toHaveAttribute(
      "href",
      "https://docs.litellm.ai/docs/proxy/lens/mcp",
    );
  });

  it("should lead with the admin token setup when Lens is opened directly", async () => {
    renderWithLens(
      <LensHostProvider host={{ surface: "standalone" }}>
        <McpConnection />
      </LensHostProvider>,
    );

    const direct = await screen.findByRole("region", { name: "Directly to Lens" });
    expect(within(direct).getByText(`${window.location.origin}/lens/mcp`)).toBeVisible();
    expect(within(direct).getByText(/--bearer-token-env-var LENS_ADMIN_TOKEN/)).toBeVisible();
    const gateway = screen.getByRole("region", { name: "Through LiteLLM" });
    expect(within(gateway).getByText(/--bearer-token-env-var LITELLM_API_KEY/)).toBeVisible();
    expect(direct.compareDocumentPosition(gateway)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
  });
});
