import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { renderWithProviders } from "../../../../tests/test-utils";
import { LensHostProvider, type LensHost } from "../../../host/LensHost";
import { copyToClipboard } from "../../../utils/dataUtils";
import { SetupAgentPrompt } from "./SetupAgentPrompt";

vi.mock("../../../utils/dataUtils", () => ({
  copyToClipboard: vi.fn().mockResolvedValue(true),
}));

beforeEach(() => vi.clearAllMocks());

it.each([
  [
    { analysis: "deployment", surface: "embedded" },
    "existing LiteLLM admin dashboard",
  ],
  [{ analysis: "deployment", surface: "standalone" }, "standalone Lens app"],
  [{}, "existing LiteLLM admin dashboard"],
] satisfies readonly (readonly [LensHost, string])[])(
  "copies the correct host context for %j",
  async (host, expected) => {
    const user = userEvent.setup();
    renderWithProviders(
      <LensHostProvider host={host}>
        <SetupAgentPrompt />
      </LensHostProvider>,
    );

    await user.click(screen.getByRole("button", { name: "Set it up for me" }));

    expect(copyToClipboard).toHaveBeenCalledWith(
      expect.stringContaining(expected),
      "Prompt copied",
    );
    expect(
      screen.getByRole("button", { name: "Set it up for me" }),
    ).toHaveTextContent("Prompt copied");
  },
);

it("copies complete instructions from the first screen and exposes the next conversation", async () => {
  const user = userEvent.setup();
  renderWithProviders(
    <SetupAgentPrompt
      goal="tracing"
      prominent
      connection={{
        connected: true,
        url: "https://lens.example",
        status: { storage_ready: true },
      }}
    />,
  );

  expect(
    screen.getByRole("heading", { name: "Give this to your coding agent" }),
  ).toBeVisible();
  expect(screen.getByText(/^Connect this project to Lens\./)).toHaveTextContent(
    "What would you like to instrument next?",
  );
  await user.click(
    screen.getByRole("button", { name: "Copy setup instructions" }),
  );
  expect(copyToClipboard).toHaveBeenCalledWith(
    expect.stringContaining("https://lens.example/v1/traces/receipt"),
    "Prompt copied",
  );
  expect(copyToClipboard).toHaveBeenCalledWith(
    expect.stringContaining(
      'Then ask: "What would you like to instrument next?"',
    ),
    "Prompt copied",
  );
  expect(
    screen.getByRole("button", { name: "Copy setup instructions" }),
  ).toHaveTextContent("Prompt copied");
});
