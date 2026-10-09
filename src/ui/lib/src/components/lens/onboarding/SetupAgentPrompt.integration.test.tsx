import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { renderWithProviders } from "../../../../tests/test-utils";
import { LensHostProvider, type LensHost } from "../../../host/LensHost";
import { copyToClipboard } from "../../../utils/dataUtils";
import { SetupAgentPrompt } from "./SetupAgentPrompt";

vi.mock("../../../utils/dataUtils", () => ({ copyToClipboard: vi.fn().mockResolvedValue(true) }));

beforeEach(() => vi.clearAllMocks());

it.each([
  [{ analysis: "deployment", surface: "embedded" }, "existing LiteLLM admin dashboard"],
  [{ analysis: "deployment", surface: "standalone" }, "standalone Lens app"],
  [{}, "existing LiteLLM admin dashboard"],
] satisfies readonly (readonly [LensHost, string])[])("copies the correct host context for %j", async (host, expected) => {
  const user = userEvent.setup();
  renderWithProviders(
    <LensHostProvider host={host}>
      <SetupAgentPrompt />
    </LensHostProvider>,
  );

  await user.click(screen.getByRole("button", { name: "Set it up for me" }));

  expect(copyToClipboard).toHaveBeenCalledWith(expect.stringContaining(expected), "Prompt copied");
  expect(screen.getByRole("button", { name: "Set it up for me" })).toHaveTextContent("Prompt copied");
});
