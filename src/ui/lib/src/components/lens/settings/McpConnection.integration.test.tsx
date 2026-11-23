import { screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { renderWithLens } from "../../../../tests/lens-test-utils";
import { LensHostProvider } from "../../../host/LensHost";
import { McpConnection } from "./McpConnection";

describe("MCP connection", () => {
  it("should show the LiteLLM key commands when Lens runs inside the gateway", async () => {
    renderWithLens(<McpConnection />);

    expect(await screen.findByText(`${window.location.origin}/lens/mcp`)).toBeVisible();
    expect(screen.getByText(/Use your LiteLLM key/)).toBeVisible();
    expect(screen.getByRole("button", { name: "Copy Claude Code command" })).toBeVisible();
    expect(screen.getByRole("button", { name: "Copy Codex command" })).toBeVisible();
    expect(screen.getByRole("link", { name: "Docs" })).toHaveAttribute(
      "href",
      "https://docs.litellm.ai/docs/proxy/lens/mcp",
    );
  });

  it("should ask for the Lens admin token when Lens is opened directly", async () => {
    renderWithLens(
      <LensHostProvider host={{ surface: "standalone" }}>
        <McpConnection />
      </LensHostProvider>,
    );

    expect(await screen.findByText(/Use LENS_ADMIN_TOKEN/)).toBeVisible();
    expect(screen.getByText(/--bearer-token-env-var LENS_ADMIN_TOKEN/)).toBeVisible();
  });
});
