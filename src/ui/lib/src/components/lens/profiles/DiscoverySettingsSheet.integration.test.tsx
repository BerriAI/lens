import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../tests/test-utils";
import { DiscoverySettingsSheet } from "./DiscoverySettingsSheet";
import type { ProfileSettingsView } from "./schemas";

const models = { discovery: "claude-haiku", embedding: "qwen-embed", classification: "typesafe/jev-latest" };

const settingsView = (overrides: Partial<ProfileSettingsView> = {}): ProfileSettingsView => ({
  settings: {
    agent: "assistant",
    enabled: false,
    models: { discovery: "", embedding: "", classification: "" },
    revision: 0,
    updated_at: null,
  },
  defaults: models,
  options: {
    discovery: ["claude-haiku", "claude-sonnet"],
    embedding: ["qwen-embed"],
    classification: ["typesafe/jev-latest"],
  },
  can_write: true,
  ...overrides,
});

const enabled = settingsView({ settings: { agent: "assistant", enabled: true, models, revision: 3, updated_at: null } });

function routes(proxy: ReturnType<typeof stubGateway>, view: ProfileSettingsView) {
  proxy.get.mockImplementation(async (path) => {
    if (path === "/lens/profiles/settings") return view;
    if (path === "/lens/profiles/taxonomy") return { taxonomy: null, active: null, discovery_available: view.settings.enabled, can_write: true };
    throw new Error(`unexpected ${path}`);
  });
  proxy.post.mockImplementation(async (path) => {
    if (path === "/lens/profiles/discovery/preview")
      return {
        eligible_runs: 140,
        sample_runs: 140,
        previews: [{ trace_id: "t1", start_ms: 1, input_preview: "Why was I charged twice?" }],
      };
    if (path === "/lens/profiles/discovery") return { taxonomy: null, active: null, discovery_available: true, can_write: true };
    throw new Error(`unexpected ${path}`);
  });
  proxy.put.mockImplementation(async (_, request) => ({ ...enabled.settings, ...(request.body as object), revision: 1 }));
}

const render = () =>
  renderWithLens(
    <DiscoverySettingsSheet agent="assistant" window={{ startMs: 0, endMs: 86_400_000 }} open onOpenChange={vi.fn()} />,
  );

const discoveryCalls = (proxy: ReturnType<typeof stubGateway>) =>
  proxy.post.mock.calls.filter(([path]) => path === "/lens/profiles/discovery");

beforeEach(() => testQueryClient.clear());
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it("opts in with the three chosen models without starting any discovery", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  routes(proxy, settingsView());
  render();

  const toggle = await screen.findByRole("switch", { name: "Use paid models for assistant" });
  expect(toggle).not.toBeChecked();
  expect(screen.getByRole("button", { name: "Discover tasks" })).toBeDisabled();
  await user.click(toggle);
  await user.click(screen.getByRole("button", { name: "Save settings" }));

  await waitFor(() =>
    expect(proxy.put).toHaveBeenCalledWith(
      "/lens/profiles/settings",
      expect.objectContaining({ body: { agent: "assistant", revision: 0, enabled: true, models } }),
    ),
  );
  expect(discoveryCalls(proxy)).toEqual([]);
});

it("explains missing gateway models and still previews the sample without model calls", async () => {
  const proxy = stubGateway();
  routes(
    proxy,
    settingsView({
      defaults: { discovery: "", embedding: "", classification: "" },
      options: { discovery: ["claude-haiku"], embedding: [], classification: [] },
    }),
  );
  render();

  expect(await screen.findByText(/No embedding models are available/)).toBeVisible();
  expect(screen.getByText(/No Jev models are available/)).toBeVisible();
  expect(await screen.findByText(/Why was I charged twice\?/)).toBeVisible();
  expect(screen.getByText(/eligible runs in this range/)).toHaveTextContent("140 eligible runs in this range");
  await userEvent.setup().click(screen.getByRole("switch", { name: "Use paid models for assistant" }));
  expect(screen.getByRole("button", { name: "Save settings" })).toBeDisabled();
  expect(screen.getByText("Choose all three models to turn discovery on.")).toBeVisible();
  expect(discoveryCalls(proxy)).toEqual([]);
});

it("starts discovery only after confirming the agent, range, sample and models", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  routes(proxy, enabled);
  render();

  fireEvent.change(await screen.findByLabelText("From"), { target: { value: "2020-01-01" } });
  fireEvent.change(screen.getByLabelText("Through"), { target: { value: "2020-01-07" } });
  fireEvent.change(screen.getByLabelText("Sample size"), { target: { value: "200" } });
  await screen.findByText(/Why was I charged twice\?/);
  await user.click(screen.getByRole("button", { name: "Discover tasks" }));

  const summary = within(screen.getByRole("alertdialog", { name: "Confirm discovery" }));
  expect(summary.getByText("assistant")).toBeVisible();
  expect(summary.getByText("140 of 140 eligible runs")).toBeVisible();
  expect(summary.getByText("claude-haiku, qwen-embed, typesafe/jev-latest")).toBeVisible();
  expect(discoveryCalls(proxy)).toEqual([]);
  await user.click(summary.getByRole("button", { name: "Start discovery" }));

  await waitFor(() => expect(discoveryCalls(proxy)).toHaveLength(1));
  expect(discoveryCalls(proxy)[0][1].body).toEqual({
    agent: "assistant",
    start_ms: new Date(2020, 0, 1).getTime(),
    end_ms: new Date(2020, 0, 8).getTime(),
    max_runs: 200,
  });
});

it("refuses to discover with unsaved changes or a sample below the minimum", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  routes(proxy, enabled);
  render();

  fireEvent.change(await screen.findByLabelText("Sample size"), { target: { value: "10" } });
  expect(await screen.findByText("Sample between 20 and 500 runs.")).toBeVisible();
  expect(screen.getByRole("button", { name: "Discover tasks" })).toBeDisabled();

  fireEvent.change(screen.getByLabelText("Sample size"), { target: { value: "300" } });
  await user.click(screen.getByRole("switch", { name: "Use paid models for assistant" }));
  expect(screen.getByText("Save your changes before starting a discovery.")).toBeVisible();
  expect(screen.getByRole("button", { name: "Discover tasks" })).toBeDisabled();
});

it("shows a read-only view to admins who cannot change settings", async () => {
  const proxy = stubGateway();
  routes(proxy, { ...enabled, can_write: false });
  render();

  expect(await screen.findByRole("switch", { name: "Use paid models for assistant" })).toHaveAttribute("data-disabled");
  expect(screen.queryByRole("button", { name: "Save settings" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Discover tasks" })).not.toBeInTheDocument();
});

it("cannot start a confirmed discovery once its settings stop matching what was saved", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  routes(proxy, enabled);
  render();

  await screen.findByText(/Why was I charged twice\?/);
  await user.click(screen.getByRole("button", { name: "Discover tasks" }));
  const start = within(screen.getByRole("alertdialog", { name: "Confirm discovery" })).getByRole("button", {
    name: "Start discovery",
  });
  expect(start).toBeEnabled();

  await user.click(screen.getByRole("switch", { name: "Use paid models for assistant" }));

  expect(start).toBeDisabled();
  expect(screen.getByText("Not saved yet. Task discovery stays on until you save settings below.")).toBeVisible();
  await user.click(start);
  expect(discoveryCalls(proxy)).toEqual([]);
});

it("turns discovery off immediately with the saved models", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  routes(proxy, enabled);
  render();

  await user.click(await screen.findByRole("button", { name: "Turn off now" }));

  await waitFor(() =>
    expect(proxy.put).toHaveBeenCalledWith(
      "/lens/profiles/settings",
      expect.objectContaining({ body: { agent: "assistant", revision: 3, enabled: false, models } }),
    ),
  );
});

it("keeps the chosen range and sample size after saving settings", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  routes(proxy, settingsView());
  render();

  fireEvent.change(await screen.findByLabelText("From"), { target: { value: "2020-01-01" } });
  fireEvent.change(screen.getByLabelText("Sample size"), { target: { value: "120" } });
  routes(proxy, { ...enabled, settings: { ...enabled.settings, revision: 1 } });
  await user.click(screen.getByRole("switch", { name: "Use paid models for assistant" }));
  await user.click(screen.getByRole("button", { name: "Save settings" }));

  expect(await screen.findByText("Task discovery is on for this agent")).toBeVisible();
  expect(screen.getByLabelText("Sample size")).toHaveValue(120);
  expect(screen.getByLabelText("From")).toHaveValue("2020-01-01");
});
