import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it } from "vitest";
import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../tests/test-utils";
import { createLensDemoData } from "../data/demo/fixtures";
import type { Lens } from "../model/types";
import { AutomaticAnalysis } from "./AutomaticAnalysis";

const fixture = createLensDemoData().lenses[0];
const initial: Lens = {
  ...fixture,
  id: "auto-agent-fixture",
  last_scan_at: null,
  jobs: [],
  findings: [],
  settings: {
    ...fixture.settings,
    enabled: true,
    model: "chat-alias",
    context: "Find errors",
    interval_minutes: 60,
  },
};
let proxy = stubGateway();
beforeEach(() => {
  testQueryClient.clear();
  proxy = stubGateway();
  proxy.get.mockImplementation(async (path) => {
    if (path === "/lens/models")
      return {
        data: [{ id: "chat-alias" }, { id: "other-chat" }, { id: "evaluation-only" }],
      };
    if (path === "/lens/model_group/info")
      return {
        data: [
          { model_group: "chat-alias", mode: "chat", providers: [] },
          { model_group: "other-chat", mode: "chat", providers: [] },
          { model_group: "evaluation-only", mode: "evaluation", providers: [] },
        ],
      };
    return { lenses: [], data: [] };
  });
  proxy.post.mockResolvedValue({ eligible: 7, selected: 7, executions: [] });
});

it("should show the first-run trace threshold with the actual received count", async () => {
  renderWithLens(<AutomaticAnalysis lenses={[initial]} ready readOnly={false} />);
  expect(await screen.findByText("Waiting for traces · 7/10")).toBeVisible();
  expect(screen.getByText("3 more traces before the first run")).toBeVisible();
  expect(proxy.post).toHaveBeenCalledWith(
    "/lens/preview/sample",
    expect.objectContaining({
      body: expect.objectContaining({
        selection: expect.objectContaining({
          agent_name: initial.settings.agent_name,
        }),
      }),
    }),
  );
});

it("should report a trace count failure without inventing a missing trace count", async () => {
  proxy.post.mockRejectedValue(new Error("Trace storage unavailable"));
  renderWithLens(<AutomaticAnalysis lenses={[initial]} ready readOnly={false} />);
  expect(await screen.findByText("Trace count unavailable")).toBeVisible();
  expect(screen.getByText("Retrying the trace count automatically")).toBeVisible();
  expect(screen.queryByText(/more traces before/)).not.toBeInTheDocument();
});

it("should show running progress instead of a countdown", () => {
  const running: Lens = {
    ...initial,
    jobs: [
      {
        ...fixture.jobs[0],
        status: "running",
        stage: "Reading traces",
        finished_at: null,
      },
    ],
  };
  renderWithLens(<AutomaticAnalysis lenses={[running]} ready readOnly={false} />);
  expect(screen.getByText("Running")).toBeVisible();
  expect(screen.getByText("Reading traces")).toBeVisible();
  expect(screen.queryByText(/more traces before/)).not.toBeInTheDocument();
  expect(proxy.post).not.toHaveBeenCalled();
});

it("should show the next scheduled time and last result after the first analysis", () => {
  const next = new Date(Date.now() + 3600000).toISOString();
  const completed: Lens = {
    ...initial,
    next_run_at: next,
    jobs: [
      {
        ...fixture.jobs[0],
        status: "completed",
        findings: [],
        finished_at: new Date().toISOString(),
      },
    ],
  };
  renderWithLens(<AutomaticAnalysis lenses={[completed]} ready readOnly={false} />);
  expect(screen.getByText(`Next run: ${new Date(next).toLocaleString()}`)).toBeVisible();
  expect(screen.getByText(/Last result: 0 findings/)).toBeVisible();
  expect(proxy.post).not.toHaveBeenCalled();
});

it("should preserve paused settings while saving a model, prompt, and frequency", async () => {
  const user = userEvent.setup();
  const paused: Lens = {
    ...initial,
    settings: { ...initial.settings, enabled: false },
  };
  proxy.put.mockImplementation(async (_path, request) => ({
    ...paused,
    settings: request.body,
  }));
  renderWithLens(<AutomaticAnalysis lenses={[paused]} ready readOnly={false} />);
  expect(screen.getByText("Paused")).toBeVisible();
  await user.click(
    screen.getByRole("button", {
      name: `Configure analysis for ${paused.settings.agent_name}`,
    }),
  );
  const dialog = screen.getByRole("dialog");
  const model = within(dialog).getByRole("combobox", {
    name: "Analysis model",
  });
  await user.click(model);
  await user.clear(model);
  await user.type(model, "other");
  await user.click(await screen.findByRole("option", { name: /other-chat/ }));
  expect(screen.queryByRole("option", { name: /evaluation-only/ })).not.toBeInTheDocument();
  await user.clear(within(dialog).getByLabelText("What to look for"));
  await user.type(within(dialog).getByLabelText("What to look for"), "Find repeated tool errors");
  await user.clear(within(dialog).getByLabelText("Check every (minutes)"));
  await user.type(within(dialog).getByLabelText("Check every (minutes)"), "30");
  await user.click(within(dialog).getByRole("button", { name: "Save changes" }));
  await waitFor(() =>
    expect(proxy.put).toHaveBeenCalledWith(
      `/lens/${paused.id}`,
      expect.objectContaining({
        body: expect.objectContaining({
          enabled: false,
          model: "other-chat",
          context: "Find repeated tool errors",
          interval_minutes: 30,
        }),
      }),
    ),
  );
});

it("should expose a connection problem and hide editing for read-only viewers", () => {
  renderWithLens(<AutomaticAnalysis lenses={[initial]} ready={false} readOnly />);
  expect(screen.getByText("Waiting for connection")).toBeVisible();
  expect(screen.queryByRole("button", { name: /Configure analysis/ })).not.toBeInTheDocument();
  expect(proxy.post).not.toHaveBeenCalled();
});

it("should keep a failed analysis error visible with its next retry time", () => {
  const next = new Date(Date.now() + 3600000).toISOString();
  const failed: Lens = {
    ...initial,
    next_run_at: next,
    jobs: [
      {
        ...fixture.jobs[0],
        status: "failed",
        error: "Provider rejected the request",
        finished_at: new Date().toISOString(),
      },
    ],
  };
  renderWithLens(<AutomaticAnalysis lenses={[failed]} ready readOnly={false} />);
  expect(screen.getByText("Last analysis failed")).toBeVisible();
  expect(screen.getByRole("alert")).toHaveTextContent("Provider rejected the request");
  expect(screen.getByText(`Next run: ${new Date(next).toLocaleString()}`)).toBeVisible();
});

it("should identify a cancelled analysis without claiming it found no problems", () => {
  const cancelled: Lens = {
    ...initial,
    jobs: [{ ...fixture.jobs[0], status: "cancelled", finished_at: new Date().toISOString() }],
  };
  renderWithLens(<AutomaticAnalysis lenses={[cancelled]} ready readOnly={false} />);
  expect(screen.getByText(/Last result: cancelled/)).toBeVisible();
  expect(screen.queryByText(/Last result: 0 findings/)).not.toBeInTheDocument();
});

it("should allow pausing when the saved model is no longer in the gateway catalogue", async () => {
  const user = userEvent.setup();
  proxy.get.mockResolvedValue({ data: [] });
  proxy.put.mockImplementation(async (_path, request) => ({ ...initial, settings: request.body }));
  renderWithLens(<AutomaticAnalysis lenses={[initial]} ready readOnly={false} />);
  await user.click(screen.getByRole("button", { name: `Configure analysis for ${initial.settings.agent_name}` }));
  const dialog = screen.getByRole("dialog");
  expect(await within(dialog).findByText(/This model is unavailable for analysis/)).toBeVisible();
  expect(within(dialog).getByRole("button", { name: "Save changes" })).toBeDisabled();
  await user.click(within(dialog).getByRole("switch", { name: "Automatic analysis" }));
  await user.click(within(dialog).getByRole("button", { name: "Save changes" }));
  await waitFor(() =>
    expect(proxy.put).toHaveBeenCalledWith(
      `/lens/${initial.id}`,
      expect.objectContaining({ body: expect.objectContaining({ enabled: false, model: initial.settings.model }) }),
    ),
  );
});

it("should flag an agent's removed model after a successful refresh even when another model remains", async () => {
  const next = new Date(Date.now() + 3600000).toISOString();
  const completed: Lens = {
    ...initial,
    next_run_at: next,
    jobs: [{ ...fixture.jobs[0], status: "completed", findings: [], finished_at: new Date().toISOString() }],
  };
  renderWithLens(<AutomaticAnalysis lenses={[completed]} ready readOnly={false} />);
  expect(await screen.findByText(`Next run: ${new Date(next).toLocaleString()}`)).toBeVisible();
  await waitFor(() => expect(proxy.get).toHaveBeenCalledWith("/lens/models", expect.anything()));
  proxy.get.mockImplementation(async (path) =>
    path === "/lens/models"
      ? { data: [{ id: "other-chat" }] }
      : { data: [{ model_group: "other-chat", mode: "chat", providers: [] }] },
  );
  await testQueryClient.refetchQueries();
  expect(await screen.findByText("Selected model unavailable")).toBeVisible();
  expect(screen.getByText(/Configure another model or pause analysis/)).toBeVisible();
  expect(screen.getByTitle("chat-alias")).toBeVisible();
  expect(screen.getByRole("button", { name: `Configure analysis for ${initial.settings.agent_name}` })).toBeEnabled();
  expect(screen.queryByText(`Next run: ${new Date(next).toLocaleString()}`)).not.toBeInTheDocument();
  expect(proxy.put).not.toHaveBeenCalled();
});

it("should not treat a transient catalogue request failure as a removed model", async () => {
  const next = new Date(Date.now() + 3600000).toISOString();
  const completed: Lens = {
    ...initial,
    next_run_at: next,
    jobs: [{ ...fixture.jobs[0], status: "completed", findings: [], finished_at: new Date().toISOString() }],
  };
  renderWithLens(<AutomaticAnalysis lenses={[completed]} ready readOnly={false} />);
  await waitFor(() => expect(proxy.get).toHaveBeenCalledWith("/lens/model_group/info", expect.anything()));
  proxy.get.mockRejectedValue(new Error("Gateway unavailable"));
  await testQueryClient.refetchQueries();
  expect(screen.getByText(`Next run: ${new Date(next).toLocaleString()}`)).toBeVisible();
  expect(screen.queryByText("Selected model unavailable")).not.toBeInTheDocument();
  expect(proxy.put).not.toHaveBeenCalled();
});
