import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../tests/test-utils";
import { LensHostProvider } from "../../../host/LensHost";
import { lensKeys } from "../data/queries";
import type { GatewayStatus, SignalConfig } from "../model/types";
import { GatewayConnection } from "./GatewayConnection";
import { SignalForm } from "./signals/SignalSettings";

const connected: GatewayStatus = {
  configured: true,
  connected: true,
  api_base: "https://gateway.example.test/v1",
  analysis_models: 2,
  evaluation_models: 0,
  error: null,
  last_refreshed: "2026-10-09T12:00:00Z",
};
const saved: SignalConfig = {
  model: "",
  threshold: 0.5,
  signals: [{ id: "user_frustration", name: "User frustration", question: "Is the user frustrated?" }],
};
const evaluationModel = { model_group: "gateway-jev", providers: ["typesafe"], mode: "evaluation" };
const chatModel = { model_group: "gateway-chat", providers: ["openai"], mode: "chat" };

function ConnectionWithSignals({ config = saved }: { config?: SignalConfig }) {
  return (
    <LensHostProvider host={{ surface: "standalone" }}>
      <GatewayConnection />
      <SignalForm saved={config} />
    </LensHostProvider>
  );
}

describe("Gateway connection", () => {
  beforeEach(() => testQueryClient.clear());

  it("should render connection status without editable credentials", async () => {
    const gateway = stubGateway();
    gateway.get.mockReturnValue(connected);
    renderWithLens(<GatewayConnection />);

    expect(await screen.findByText("Connected to LiteLLM")).toBeVisible();
    expect(screen.getByText(connected.api_base!)).toBeVisible();
    expect(screen.getByText("2 analysis models · 0 evaluation models")).toBeVisible();
    expect(screen.getByText(/No evaluation models found/)).toBeVisible();
    expect(screen.getByText(/Last refreshed/)).toBeVisible();
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
    expect(gateway.post).not.toHaveBeenCalled();
  });

  it("should refresh the shared catalogue and save the discovered evaluation model", async () => {
    const user = userEvent.setup();
    const gateway = stubGateway();
    gateway.get.mockImplementation((path) => {
      if (path === "/lens/gateway") return connected;
      if (path === "/lens/model_group/info") return { data: [chatModel] };
      throw new Error(`Unexpected request: ${path}`);
    });
    gateway.post.mockReturnValue({ ...connected, evaluation_models: 1 });
    gateway.put.mockImplementation((_path, request) => request.body);
    testQueryClient.setQueryData(lensKeys.models("test"), { data: [{ id: "gateway-chat" }] });
    renderWithLens(<ConnectionWithSignals />);

    expect(await screen.findByText("Connected to LiteLLM")).toBeVisible();
    expect(screen.getByText(/Your gateway has no evaluation models/)).toBeVisible();
    await user.click(screen.getByRole("combobox", { name: "System 1 model" }));
    expect(await screen.findByText("No evaluation models configured for Lens")).toBeVisible();
    expect(screen.queryByRole("option", { name: /gateway-chat/ })).not.toBeInTheDocument();
    await user.keyboard("{Escape}");

    gateway.get.mockImplementation((path) => {
      if (path === "/lens/gateway") return { ...connected, evaluation_models: 1 };
      if (path === "/lens/model_group/info") return { data: [chatModel, evaluationModel] };
      throw new Error(`Unexpected request: ${path}`);
    });
    await user.click(screen.getByRole("button", { name: "Refresh models" }));

    expect(await screen.findByText("2 analysis models · 1 evaluation models")).toBeVisible();
    await waitFor(() => expect(screen.getByRole("button", { name: "Refresh models" })).toBeEnabled());
    expect(gateway.post).toHaveBeenCalledWith(
      "/lens/gateway/refresh",
      expect.objectContaining({
        authorization: "Bearer test",
        body: undefined,
      }),
    );
    expect(testQueryClient.getQueryState(lensKeys.models("test"))?.isInvalidated).toBe(true);
    await user.click(screen.getByRole("combobox", { name: "System 1 model" }));
    await user.click(await screen.findByRole("option", { name: /gateway-jev/ }));
    expect(screen.queryByRole("option", { name: /gateway-chat/ })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Save signals" }));

    expect(await screen.findByText("Saved")).toBeVisible();
    expect(gateway.put).toHaveBeenCalledWith(
      "/lens/signals",
      expect.objectContaining({
        body: { ...saved, model: evaluationModel.model_group },
      }),
    );
  });

  it("should keep a partially discovered gateway connected and show the metadata warning separately", async () => {
    const gateway = stubGateway();
    const warning = "1 model was skipped because its metadata is missing";
    gateway.get.mockReturnValue({ ...connected, error: warning });
    renderWithLens(<GatewayConnection />);

    expect(await screen.findByText("Connected to LiteLLM")).toBeVisible();
    expect(screen.getByText(warning)).toBeVisible();
    expect(screen.getByText("2 analysis models · 0 evaluation models")).toBeVisible();
    expect(screen.getByRole("button", { name: "Refresh models" })).toBeEnabled();
    expect(screen.queryByText("Gateway connection unavailable")).not.toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("should retain the saved model and dirty form when gateway refresh fails", async () => {
    const user = userEvent.setup();
    const gateway = stubGateway();
    gateway.get.mockImplementation((path) =>
      path === "/lens/gateway" ? { ...connected, evaluation_models: 1 } : { data: [evaluationModel] },
    );
    gateway.post.mockReturnValue({
      ...connected,
      connected: false,
      evaluation_models: 1,
      error: "The gateway could not be reached",
    });
    renderWithLens(<ConnectionWithSignals config={{ ...saved, model: evaluationModel.model_group }} />);
    expect(await screen.findByText("Connected to LiteLLM")).toBeVisible();
    const model = screen.getByRole("combobox", { name: "System 1 model" });
    const threshold = screen.getByRole("spinbutton", { name: "Flag at score" });
    act(() => fireEvent.change(threshold, { target: { value: "70" } }));
    await user.click(screen.getByRole("button", { name: "Refresh models" }));

    expect(await screen.findByText("Gateway connection unavailable")).toBeVisible();
    expect(within(screen.getByRole("region", { name: "Model gateway" })).getByRole("alert")).toHaveTextContent(
      "The gateway could not be reached",
    );
    expect(screen.getByText(/Could not load System 1 models/)).toBeVisible();
    expect(screen.queryByText(/Your gateway has no evaluation models/)).not.toBeInTheDocument();
    expect(screen.getByText(/Last available catalogue/)).toBeVisible();
    expect(model).toHaveValue(evaluationModel.model_group);
    expect(threshold).toHaveValue(70);
    expect(gateway.put).not.toHaveBeenCalled();

    gateway.post.mockReturnValue({ ...connected, evaluation_models: 1 });
    await user.click(screen.getByRole("button", { name: "Refresh models" }));
    expect(await screen.findByText("Connected to LiteLLM")).toBeVisible();
    expect(screen.queryByText(/Could not load System 1 models/)).not.toBeInTheDocument();
    expect(model).toHaveValue(evaluationModel.model_group);
    expect(threshold).toHaveValue(70);
  });

  it("should retry a failed connection check without calling the refresh mutation", async () => {
    const user = userEvent.setup();
    const gateway = stubGateway();
    gateway.get.mockImplementation(() => {
      throw new Error("Connection check unavailable");
    });
    renderWithLens(<GatewayConnection />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Connection check unavailable");
    expect(screen.queryByText("No model gateway configured")).not.toBeInTheDocument();
    gateway.get.mockReturnValue(connected);
    await user.click(screen.getByRole("button", { name: "Retry connection check" }));
    expect(await screen.findByText("Connected to LiteLLM")).toBeVisible();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(gateway.post).not.toHaveBeenCalled();
  });

  it("should allow choosing a known evaluation model while gateway discovery is unavailable", async () => {
    const user = userEvent.setup();
    const gateway = stubGateway();
    gateway.get.mockImplementation((path) =>
      path === "/lens/gateway"
        ? { ...connected, connected: false, error: "The gateway could not be reached" }
        : { data: [evaluationModel] },
    );
    gateway.put.mockImplementation((_path, request) => request.body);
    renderWithLens(<ConnectionWithSignals />);

    expect(await screen.findByText(/Could not load System 1 models/)).toBeVisible();
    await user.click(screen.getByRole("combobox", { name: "System 1 model" }));
    await user.click(await screen.findByRole("option", { name: /gateway-jev/ }));
    expect(screen.getByRole("button", { name: "Save signals" })).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "Save signals" }));
    expect(await screen.findByText("Saved")).toBeVisible();
    expect(gateway.put).toHaveBeenCalledWith(
      "/lens/signals",
      expect.objectContaining({
        body: { ...saved, model: evaluationModel.model_group },
      }),
    );
  });

  it.each([{ readOnly: true }, { canInvestigate: false }])(
    "should show status without refresh permission %j",
    async (onboarding) => {
      const gateway = stubGateway();
      gateway.get.mockReturnValue(connected);
      renderWithLens(<GatewayConnection />, { onboarding });
      expect(await screen.findByText("Connected to LiteLLM")).toBeVisible();
      expect(screen.queryByRole("button", { name: "Refresh models" })).not.toBeInTheDocument();
      expect(gateway.post).not.toHaveBeenCalled();
    },
  );

  it("should explain server setup when the optional gateway is unconfigured", async () => {
    const gateway = stubGateway();
    gateway.get.mockReturnValue({
      ...connected,
      configured: false,
      connected: false,
      api_base: null,
      last_refreshed: null,
      analysis_models: 0,
      evaluation_models: 0,
    });
    renderWithLens(<GatewayConnection />);
    expect(await screen.findByText("No model gateway configured")).toBeVisible();
    expect(screen.getByRole("link", { name: "Configure gateway" })).toHaveAttribute(
      "href",
      expect.stringContaining("docs/analysis.md"),
    );
    expect(screen.queryByRole("button", { name: "Refresh models" })).not.toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
});
