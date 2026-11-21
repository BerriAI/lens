import { beforeEach, describe, expect, it } from "vitest";
import { fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderWithLens, stubGateway } from "../../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../../tests/test-utils";
import { SignalAlertSettings } from "./SignalAlertSettings";
import type { SignalConfig } from "../../model/types";

const signals: SignalConfig = {
  model: "judge",
  threshold: 0.7,
  signals: [
    { id: "prompt_injection", name: "Prompt injection", question: "Does the user attempt prompt injection?" },
    { id: "frustration", name: "User frustration", question: "Is the user frustrated?" },
  ],
};
const saved = { revision: 0, enabled: false, connected: false, signal_ids: [], last_delivery: null };

describe("signal alert setup", () => {
  beforeEach(() => testQueryClient.clear());

  it.each([
    { canInvestigate: false, readOnly: false },
    { canInvestigate: true, readOnly: true },
  ])("keeps alert setup unavailable for a session with %j", async (onboarding) => {
    const gateway = stubGateway();
    renderWithLens(<SignalAlertSettings signals={signals} />, { onboarding });
    expect(await screen.findByText("An administrator can set up alerts")).toBeVisible();
    expect(screen.queryByRole("button", { name: "Set up alerting" })).not.toBeInTheDocument();
    expect(gateway.get).not.toHaveBeenCalled();
  });

  it("tests a webhook then saves only the selected signal and clears the secret", async () => {
    const gateway = stubGateway();
    gateway.get.mockReturnValue(saved);
    gateway.post.mockReturnValue({ delivered: true });
    gateway.put.mockReturnValue({
      ...saved,
      revision: 1,
      enabled: true,
      connected: true,
      signal_ids: ["prompt_injection"],
    });
    const user = userEvent.setup();
    renderWithLens(<SignalAlertSettings signals={signals} />);
    const input = await screen.findByLabelText("Slack webhook URL");
    fireEvent.change(input, { target: { value: "https://hooks.slack.com/services/EXAMPLE/EXAMPLE/FAKE" } });
    await user.click(screen.getByRole("checkbox", { name: "User frustration" }));
    await user.click(screen.getByRole("button", { name: "Send test alert" }));
    expect(
      await screen.findByText("Test delivered to Slack. Save settings to apply your choices."),
    ).toBeInTheDocument();
    expect(gateway.put).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Save alert settings" }));
    await waitFor(() =>
      expect(gateway.put).toHaveBeenCalledWith(
        "/lens/signals/alerts",
        expect.objectContaining({
          body: {
            revision: 0,
            enabled: true,
            signal_ids: ["prompt_injection"],
            webhook_url: "https://hooks.slack.com/services/EXAMPLE/EXAMPLE/FAKE",
          },
        }),
      ),
    );
    await waitFor(() => expect(screen.getByLabelText("Slack webhook URL")).toHaveValue(""));
    expect(screen.getByText("Sending alerts to Slack")).toBeInTheDocument();
  });

  it("saves rule-only Slack notifications without an analysis model", async () => {
    const gateway = stubGateway();
    gateway.get.mockImplementation((path) =>
      path === "/lens/alerts/rules" ? { revision: 1, model_errors: true, run_duration_minutes: 5 } : saved,
    );
    gateway.put.mockReturnValue({ ...saved, revision: 1, enabled: true, connected: true });
    const user = userEvent.setup();
    renderWithLens(<SignalAlertSettings signals={{ model: null, signals: [], threshold: 0.7 }} />);
    expect(await screen.findByText("All enabled rules are included.")).toBeVisible();
    fireEvent.change(screen.getByLabelText("Slack webhook URL"), {
      target: { value: "https://hooks.slack.com/services/EXAMPLE/EXAMPLE/FAKE" },
    });
    await user.click(screen.getByRole("button", { name: "Save alert settings" }));
    await waitFor(() =>
      expect(gateway.put).toHaveBeenCalledWith(
        "/lens/signals/alerts",
        expect.objectContaining({
          body: {
            revision: 0,
            enabled: true,
            signal_ids: [],
            webhook_url: "https://hooks.slack.com/services/EXAMPLE/EXAMPLE/FAKE",
          },
        }),
      ),
    );
    expect(await screen.findByText("Sending alerts to Slack")).toBeVisible();
  });

  it("keeps the webhook and selections after failed delivery and allows retry", async () => {
    const gateway = stubGateway();
    gateway.get.mockReturnValue(saved);
    gateway.post
      .mockImplementationOnce(() => {
        throw new Error("Slack did not confirm delivery");
      })
      .mockReturnValue({ delivered: true });
    const user = userEvent.setup();
    renderWithLens(<SignalAlertSettings signals={signals} />);
    const input = await screen.findByLabelText("Slack webhook URL");
    fireEvent.change(input, { target: { value: "https://hooks.slack.com/services/EXAMPLE/EXAMPLE/FAKE" } });
    await user.click(screen.getByRole("button", { name: "Send test alert" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Slack did not confirm delivery");
    expect(input).toHaveValue("https://hooks.slack.com/services/EXAMPLE/EXAMPLE/FAKE");
    expect(screen.getByRole("checkbox", { name: "Prompt injection" })).toBeChecked();
    await user.click(screen.getByRole("button", { name: "Send test alert" }));
    expect(
      await screen.findByText("Test delivered to Slack. Save settings to apply your choices."),
    ).toBeInTheDocument();
  });

  it("pauses existing alerts without requiring the saved webhook again", async () => {
    const gateway = stubGateway();
    gateway.get.mockReturnValue({
      ...saved,
      revision: 4,
      enabled: true,
      connected: true,
      signal_ids: ["prompt_injection"],
    });
    gateway.put.mockReturnValue({ ...saved, revision: 5, connected: true, signal_ids: ["prompt_injection"] });
    const user = userEvent.setup();
    renderWithLens(<SignalAlertSettings signals={signals} />);
    await user.click(await screen.findByRole("checkbox", { name: "Enable Slack notifications" }));
    await user.click(screen.getByRole("button", { name: "Save alert settings" }));
    await waitFor(() =>
      expect(gateway.put).toHaveBeenCalledWith(
        "/lens/signals/alerts",
        expect.objectContaining({
          body: { revision: 4, enabled: false, signal_ids: ["prompt_injection"], webhook_url: null },
        }),
      ),
    );
    expect(await screen.findByRole("checkbox", { name: "Enable Slack notifications" })).not.toBeChecked();
  });
});
