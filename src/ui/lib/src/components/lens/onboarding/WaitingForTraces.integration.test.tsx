import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../tests/test-utils";
import { copyToClipboard } from "../../../utils/dataUtils";
import { WaitingForTraces } from "./WaitingForTraces";

vi.mock("../../../utils/dataUtils", () => ({ copyToClipboard: vi.fn().mockResolvedValue(true) }));

beforeEach(() => {
  testQueryClient.clear();
  vi.clearAllMocks();
});

describe("Waiting for traces", () => {
  it("should keep the generated key out of both coding-agent setup commands", async () => {
    const user = userEvent.setup();
    const gateway = stubGateway();
    const secret = "lens-trace-generated-secret-for-test";
    gateway.get.mockReturnValue({
      url: "https://lens.example/",
      connected: true,
      status: { storage_ready: true, credentials_ready: true },
    });
    gateway.post.mockReturnValue({ key: secret, active: true });
    renderWithLens(
      <WaitingForTraces accessToken="test" canMintTracingKey readOnly={false} checking={false} onCheck={vi.fn()} />,
    );

    expect(await screen.findByRole("heading", { name: "Connect your project" })).toBeVisible();
    await user.click(screen.getByText("Connection details", { selector: "summary" }));
    await user.click(screen.getByRole("button", { name: "Generate tracing key" }));
    await screen.findByText("Your tracing key");
    expect(screen.getByRole("region", { name: "Waiting for traces" })).not.toHaveTextContent(secret);

    await user.click(screen.getByRole("button", { name: "Copy setup command" }));
    const claude = vi.mocked(copyToClipboard).mock.lastCall?.[0];
    expect(claude).toMatch(/^claude '/);
    expect(claude).toContain("https://lens.example/v1/traces");
    expect(claude).toContain("<dedicated Lens tracing key>");
    expect(claude).not.toContain(secret);

    await user.click(screen.getByRole("tab", { name: "Codex" }));
    await user.click(screen.getByRole("button", { name: "Copy setup command" }));
    const codex = vi.mocked(copyToClipboard).mock.lastCall?.[0];
    expect(codex).toMatch(/^codex '/);
    expect(codex).toContain("<dedicated Lens tracing key>");
    expect(codex).not.toContain(secret);
  });
});
