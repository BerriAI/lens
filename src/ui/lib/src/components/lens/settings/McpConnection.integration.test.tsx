import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { renderWithProviders } from "../../../../tests/test-utils";
import { McpConnection } from "./McpConnection";

it("takes existing Settings users to Lens MCP in Connectors", async () => {
  const user = userEvent.setup();
  const onUrlUpdate = vi.fn();
  renderWithProviders(<McpConnection />, {
    searchParams: "?tab=settings",
    onUrlUpdate,
  });
  await user.click(screen.getByRole("button", { name: "Open Connectors" }));
  await waitFor(() =>
    expect(onUrlUpdate.mock.lastCall?.[0].queryString).toContain(
      "tab=connectors",
    ),
  );
  expect(onUrlUpdate.mock.lastCall?.[0].queryString).toContain("connector=mcp");
});
