import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../tests/test-utils";
import { DiscoverySettingsSheet } from "./DiscoverySettingsSheet";
import type { DiscoveryPreview, ProfileSettings, ProfileSettingsView, Taxonomy, TaxonomyView } from "./schemas";

const DAY = 86_400_000;
const models = { discovery: "claude-haiku", embedding: "qwen-embed", classification: "typesafe/jev-latest" };
const none = { discovery: "", embedding: "", classification: "" };

const settingsView = (overrides: Partial<ProfileSettingsView> = {}): ProfileSettingsView => ({
  settings: { agent: "assistant", enabled: false, models: none, revision: 0, updated_at: null },
  defaults: models,
  options: {
    discovery: ["claude-haiku", "claude-sonnet"],
    embedding: ["qwen-embed"],
    classification: ["typesafe/jev-latest"],
  },
  can_write: true,
  ...overrides,
});

const configured = (settings: Partial<ProfileSettings> = {}) =>
  settingsView({ settings: { agent: "assistant", enabled: true, models, revision: 3, updated_at: null, ...settings } });

const taxonomy = (overrides: Partial<Taxonomy> = {}): Taxonomy => ({
  taxonomy_id: "tax",
  agent: "assistant",
  version: 2,
  revision: 7,
  status: "draft",
  tasks: [
    {
      task_id: "debug",
      name: "Debug failing tests",
      description: "Fix a failing test",
      inclusion_criteria: "Asks to fix a test",
      exclusion_criteria: "New features",
      required_capabilities: [],
      example_trace_ids: [],
    },
  ],
  points: [],
  job: null,
  discovered_at: "2026-11-02T12:00:00Z",
  frozen_at: null,
  error: null,
  models: { distill: "claude-haiku", embedding: "qwen-embed", synthesis: "claude-haiku", decision: "typesafe/jev-latest" },
  discovery: { start_ms: 10 * DAY, end_ms: 17 * DAY, requested_runs: 300, sampled_runs: 120 },
  windows: [],
  ...overrides,
});

const active = (overrides: Partial<Taxonomy> = {}) =>
  taxonomy({
    status: "frozen",
    revision: 3,
    tasks: [{ ...taxonomy().tasks[0], task_id: "billing", name: "Answer billing questions" }],
    windows: [{ start_ms: 20 * DAY, end_ms: null }],
    ...overrides,
  });

const discovering = (overrides: Partial<Taxonomy> = {}) =>
  taxonomy({
    status: "discovering",
    tasks: [],
    discovery: null,
    discovered_at: null,
    job: { job_id: "job", requested_at: "", lease_until: null, attempts: 1, max_runs: 300, start_ms: 0, end_ms: DAY },
    ...overrides,
  });

const view = (overrides: Partial<TaxonomyView> = {}): TaxonomyView => ({
  taxonomy: null,
  active: null,
  discovery_available: true,
  can_write: true,
  ...overrides,
});

const counted = (eligible_runs: number, sample_runs = Math.min(eligible_runs, 300)): DiscoveryPreview => ({
  eligible_runs,
  sample_runs,
  previews: [],
});

interface Server {
  settings: ProfileSettingsView;
  taxonomy: TaxonomyView;
  preview: DiscoveryPreview;
  written: TaxonomyView;
}

function serve(proxy: ReturnType<typeof stubGateway>, initial: Partial<Server> = {}): Server {
  const server: Server = {
    settings: configured(),
    taxonomy: view(),
    preview: counted(140),
    written: view(),
    ...initial,
  };
  proxy.get.mockImplementation(async (path) => {
    if (path === "/lens/profiles/settings") return server.settings;
    if (path === "/lens/profiles/taxonomy") return server.taxonomy;
    throw new Error(`unexpected ${path}`);
  });
  proxy.post.mockImplementation(async (path) => {
    if (path === "/lens/profiles/discovery/preview") return server.preview;
    if (path === "/lens/gateway/refresh") return {};
    return server.written;
  });
  proxy.put.mockImplementation(async (_, request) => ({
    ...server.settings.settings,
    ...(request.body as object),
    revision: server.settings.settings.revision + 1,
  }));
  return server;
}

const render = () =>
  renderWithLens(
    <DiscoverySettingsSheet agent="assistant" window={{ startMs: 0, endMs: 86_400_000 }} open onOpenChange={vi.fn()} />,
  );

const posts = (proxy: ReturnType<typeof stubGateway>, path: string) =>
  proxy.post.mock.calls.filter(([called]) => called === path).map(([, request]) => request.body);

const inferencePaths = ["/lens/profiles/discovery", "/lens/profiles/taxonomy/activate", "/lens/profiles/taxonomy/resume"];
const inference = (proxy: ReturnType<typeof stubGateway>) =>
  proxy.post.mock.calls.filter(([path]) => inferencePaths.includes(path));

const choose = async (user: ReturnType<typeof userEvent.setup>, role: string, model: string) => {
  await user.click(screen.getByRole("combobox", { name: role }));
  await user.click(await screen.findByRole("option", { name: model }));
};

beforeEach(() => testQueryClient.clear());
afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it("saves the three chosen models with one eligible run without calling a model", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  serve(proxy, { settings: settingsView(), preview: counted(1) });
  render();

  expect(await screen.findByText("1 of 20 eligible runs collected")).toBeVisible();
  expect(screen.getByRole("button", { name: "Discover categories" })).toBeDisabled();
  await user.click(screen.getByRole("button", { name: "Save models" }));

  await waitFor(() =>
    expect(proxy.put).toHaveBeenCalledWith(
      "/lens/profiles/settings",
      expect.objectContaining({ body: { agent: "assistant", revision: 0, enabled: true, models } }),
    ),
  );
  expect(await screen.findByText("Saved")).toBeVisible();
  expect(screen.getByRole("button", { name: "Discover categories" })).toBeDisabled();
  expect(inference(proxy)).toEqual([]);
});

it("keeps discovery unavailable at 19 eligible runs and allows it at 20 after refreshing the count", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  const server = serve(proxy, { preview: counted(19) });
  render();

  expect(await screen.findByText("19 of 20 eligible runs collected")).toBeVisible();
  expect(screen.getByRole("button", { name: "Discover categories" })).toBeDisabled();
  server.preview = counted(20);
  await user.click(screen.getByRole("button", { name: "Refresh count" }));

  expect(await screen.findByText("20 eligible runs. Discovery reads the most recent 20.")).toBeVisible();
  expect(screen.getByRole("button", { name: "Discover categories" })).toBeEnabled();
  expect(inference(proxy)).toEqual([]);
});

it("moves the cutoff of a range ending today forward when the count is refreshed", async () => {
  const opened = new Date(2026, 10, 18, 12).getTime();
  const later = opened + 3_600_000;
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(opened);
  const user = userEvent.setup();
  const proxy = stubGateway();
  const server = serve(proxy, { preview: counted(1) });
  render();
  fireEvent.change(await screen.findByLabelText("From"), { target: { value: "2026-11-17" } });
  fireEvent.change(screen.getByLabelText("Through"), { target: { value: "2026-11-18" } });
  expect(await screen.findByText("1 of 20 eligible runs collected")).toBeVisible();
  await waitFor(() =>
    expect(posts(proxy, "/lens/profiles/discovery/preview").at(-1)).toEqual(expect.objectContaining({ end_ms: opened })),
  );

  vi.setSystemTime(later);
  server.preview = counted(1000, 300);
  await user.click(screen.getByRole("button", { name: "Refresh count" }));

  expect(await screen.findByText("1,000 eligible runs. Discovery reads the most recent 300.")).toBeVisible();
  expect(posts(proxy, "/lens/profiles/discovery/preview").at(-1)).toEqual(
    expect.objectContaining({ start_ms: new Date(2026, 10, 17).getTime(), end_ms: later }),
  );
  expect(inference(proxy)).toEqual([]);
});

it("keeps the exact end of a historical range when the count is refreshed", async () => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date(2026, 10, 18, 12));
  const user = userEvent.setup();
  const proxy = stubGateway();
  const server = serve(proxy, { preview: counted(1) });
  render();
  fireEvent.change(await screen.findByLabelText("From"), { target: { value: "2026-11-09" } });
  fireEvent.change(screen.getByLabelText("Through"), { target: { value: "2026-11-10" } });
  expect(await screen.findByText("1 of 20 eligible runs collected")).toBeVisible();
  const bound = { start_ms: new Date(2026, 10, 9).getTime(), end_ms: new Date(2026, 10, 11).getTime() };
  await waitFor(() => expect(posts(proxy, "/lens/profiles/discovery/preview").at(-1)).toEqual(expect.objectContaining(bound)));
  const before = posts(proxy, "/lens/profiles/discovery/preview").length;

  vi.setSystemTime(new Date(2026, 10, 18, 13));
  server.preview = counted(25);
  await user.click(screen.getByRole("button", { name: "Refresh count" }));

  expect(await screen.findByText("25 eligible runs. Discovery reads the most recent 25.")).toBeVisible();
  expect(posts(proxy, "/lens/profiles/discovery/preview").slice(before)).toEqual([expect.objectContaining(bound)]);
});

it("shows a saved choice as saved while the settings are read again", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  serve(proxy, { settings: settingsView() });
  render();
  expect(await screen.findByText("Not saved yet. Saving does not call any model.")).toBeVisible();
  const read = proxy.get.getMockImplementation();
  proxy.get.mockImplementation(async (path, request) =>
    path === "/lens/profiles/settings" ? new Promise(() => {}) : read?.(path, request),
  );

  await user.click(screen.getByRole("button", { name: "Save models" }));

  expect(await screen.findByText("Saved")).toBeVisible();
  expect(screen.queryByText(/Not saved yet/)).not.toBeInTheDocument();
});

it("explains missing gateway models with provider neutral roles and refuses to save an incomplete choice", async () => {
  const proxy = stubGateway();
  serve(proxy, {
    settings: settingsView({ defaults: none, options: { discovery: ["claude-haiku"], embedding: [], classification: [] } }),
  });
  render();

  expect(await screen.findByText("No embedding models are available from your gateway.")).toBeVisible();
  expect(screen.getByText("No decision models are available from your gateway.")).toBeVisible();
  expect(screen.getByRole("combobox", { name: "Decision model" })).toBeVisible();
  expect(screen.queryByText(/Jev|Noul|Choice with/)).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save models" })).toBeDisabled();
  expect(screen.getByText("Choose all three models.")).toBeVisible();
  expect(proxy.put).not.toHaveBeenCalled();
});

it("saves models the gateway lists after a refresh for each role without starting discovery", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  const missing = settingsView({ defaults: none, options: { discovery: ["claude-haiku"], embedding: [], classification: [] } });
  const server = serve(proxy, { settings: missing });
  const write = proxy.post.getMockImplementation();
  proxy.post.mockImplementation(async (path, request) => {
    if (path === "/lens/gateway/refresh") server.settings = settingsView({ defaults: none });
    return write?.(path, request);
  });
  render();

  expect(await screen.findByText("No embedding models are available from your gateway.")).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Refresh models" }));
  await waitFor(() => expect(screen.queryByText(/No embedding models are available/)).not.toBeInTheDocument());
  await choose(user, "Discovery model", "claude-sonnet");
  await choose(user, "Embedding model", "qwen-embed");
  await choose(user, "Decision model", "typesafe/jev-latest");
  await user.click(screen.getByRole("button", { name: "Save models" }));

  await waitFor(() =>
    expect(proxy.put).toHaveBeenCalledWith(
      "/lens/profiles/settings",
      expect.objectContaining({
        body: { agent: "assistant", revision: 0, enabled: true, models: { ...models, discovery: "claude-sonnet" } },
      }),
    ),
  );
  expect(inference(proxy)).toEqual([]);
});

it("starts discovery only after confirming the range, sample and models", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  serve(proxy);
  render();

  fireEvent.change(await screen.findByLabelText("From"), { target: { value: "2020-01-01" } });
  fireEvent.change(screen.getByLabelText("Through"), { target: { value: "2020-01-07" } });
  fireEvent.change(screen.getByLabelText("Sample size"), { target: { value: "200" } });
  await screen.findByText("140 eligible runs. Discovery reads the most recent 140.");
  await user.click(screen.getByRole("button", { name: "Discover categories" }));

  const summary = within(screen.getByRole("region", { name: "Confirm discovery" }));
  expect(summary.getByText("140 of 140 eligible runs")).toBeVisible();
  expect(summary.getByText("claude-haiku, qwen-embed, typesafe/jev-latest")).toBeVisible();
  expect(inference(proxy)).toEqual([]);
  await user.click(summary.getByRole("button", { name: "Start discovery" }));

  await waitFor(() => expect(posts(proxy, "/lens/profiles/discovery")).toHaveLength(1));
  expect(posts(proxy, "/lens/profiles/discovery")[0]).toEqual({
    agent: "assistant",
    start_ms: new Date(2020, 0, 1).getTime(),
    end_ms: new Date(2020, 0, 8).getTime(),
    max_runs: 200,
    revision: 0,
  });
});

it("rediscovers from 1,000 eligible runs with the bounded sample while the active categories keep classifying", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  const running = active();
  serve(proxy, {
    taxonomy: view({ active: running }),
    preview: counted(1000, 300),
    written: view({ active: running, taxonomy: discovering({ revision: 1 }) }),
  });
  render();

  expect(await screen.findByText("1,000 eligible runs. Discovery reads the most recent 300.")).toBeVisible();
  expect(screen.getByText(/Discovered .* from 120 runs/)).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Rediscover categories" }));
  const summary = within(screen.getByRole("region", { name: "Confirm discovery" }));
  expect(summary.getByText("300 of 1,000 eligible runs")).toBeVisible();
  await user.click(summary.getByRole("button", { name: "Start discovery" }));

  expect(await screen.findByRole("button", { name: "Cancel discovery" })).toBeVisible();
  expect(posts(proxy, "/lens/profiles/discovery")).toEqual([expect.objectContaining({ max_runs: 300, revision: 0 })]);
  const current = within(screen.getByRole("region", { name: "Active categories" }));
  expect(current.getByText("Answer billing questions")).toBeVisible();
  expect(current.getByText(/Classifying new runs since/)).toBeVisible();
  expect(posts(proxy, "/lens/profiles/taxonomy/reset")).toEqual([]);
  expect(proxy.put).not.toHaveBeenCalled();
});

it("refuses a sample above the maximum", async () => {
  const proxy = stubGateway();
  serve(proxy);
  render();

  fireEvent.change(await screen.findByLabelText("Sample size"), { target: { value: "501" } });

  expect(await screen.findByText("Sample between 20 and 500 runs.")).toBeVisible();
  expect(screen.getByRole("button", { name: "Discover categories" })).toBeDisabled();
});

it("starts a rediscovery at the revision of the draft it replaces", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  serve(proxy, { taxonomy: view({ taxonomy: taxonomy() }) });
  render();
  await screen.findByText("140 eligible runs. Discovery reads the most recent 140.");

  await user.click(screen.getByRole("button", { name: "Rediscover categories" }));
  await user.click(within(screen.getByRole("region", { name: "Confirm discovery" })).getByRole("button", { name: "Start discovery" }));

  await waitFor(() => expect(posts(proxy, "/lens/profiles/discovery")).toHaveLength(1));
  expect(posts(proxy, "/lens/profiles/discovery")[0]).toEqual(expect.objectContaining({ revision: 7 }));
});

it("refuses to discover with unsaved model changes or a sample below the minimum", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  serve(proxy);
  render();

  fireEvent.change(await screen.findByLabelText("Sample size"), { target: { value: "10" } });
  expect(await screen.findByText("Sample between 20 and 500 runs.")).toBeVisible();
  expect(screen.getByRole("button", { name: "Discover categories" })).toBeDisabled();

  fireEvent.change(screen.getByLabelText("Sample size"), { target: { value: "300" } });
  await choose(user, "Discovery model", "claude-sonnet");
  expect(screen.getByText("Save your model changes before discovering categories.")).toBeVisible();
  expect(screen.getByRole("button", { name: "Discover categories" })).toBeDisabled();
});

it("shows a read-only view to admins who cannot change settings", async () => {
  const proxy = stubGateway();
  serve(proxy, { settings: { ...configured(), can_write: false }, taxonomy: view({ active: active(), can_write: false }) });
  render();

  expect(await screen.findByText("Answer billing questions")).toBeVisible();
  expect(screen.getByRole("combobox", { name: "Decision model" })).toBeDisabled();
  expect(screen.queryByRole("button", { name: "Save models" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Rediscover categories" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Pause classification" })).not.toBeInTheDocument();
});

it("cannot start a confirmed discovery once its models stop matching what was saved", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  serve(proxy);
  render();

  await screen.findByText("140 eligible runs. Discovery reads the most recent 140.");
  await user.click(screen.getByRole("button", { name: "Discover categories" }));
  const start = within(screen.getByRole("region", { name: "Confirm discovery" })).getByRole("button", {
    name: "Start discovery",
  });
  expect(start).toBeEnabled();

  await choose(user, "Discovery model", "claude-sonnet");

  expect(start).toBeDisabled();
  await user.click(start);
  expect(inference(proxy)).toEqual([]);
});

it("keeps the chosen range and sample size after saving models", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  serve(proxy, { settings: settingsView() });
  render();

  fireEvent.change(await screen.findByLabelText("From"), { target: { value: "2020-01-01" } });
  fireEvent.change(screen.getByLabelText("Sample size"), { target: { value: "120" } });
  await user.click(screen.getByRole("button", { name: "Save models" }));

  expect(await screen.findByText("Saved")).toBeVisible();
  expect(screen.getByLabelText("Sample size")).toHaveValue(120);
  expect(screen.getByLabelText("From")).toHaveValue("2020-01-01");
});

it("moves keyboard focus into the confirmation and back to Discover categories on Escape", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  serve(proxy);
  render();

  await screen.findByText("140 eligible runs. Discovery reads the most recent 140.");
  const discover = screen.getByRole("button", { name: "Discover categories" });
  discover.focus();
  await user.keyboard("{Enter}");

  expect(screen.getByRole("button", { name: "Start discovery" })).toHaveFocus();
  expect(discover).toHaveAttribute("aria-expanded", "true");
  await user.keyboard("{Escape}");

  expect(screen.queryByRole("region", { name: "Confirm discovery" })).not.toBeInTheDocument();
  expect(discover).toHaveFocus();
  expect(inference(proxy)).toEqual([]);
});

it("starts classifying new runs without historical backfill unless it is chosen", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  serve(proxy, { taxonomy: view({ taxonomy: taxonomy() }), written: view({ active: active({ revision: 8 }) }) });
  render();

  const discovered = within(await screen.findByRole("region", { name: "Discovered categories" }));
  expect(discovered.getByText("Debug failing tests")).toBeVisible();
  expect(discovered.queryByRole("textbox")).not.toBeInTheDocument();
  const backfill = discovered.getByRole("checkbox", { name: /Also classify earlier runs/ });
  expect(backfill).not.toBeChecked();
  expect(await discovered.findByText("At least 140 runs, one decision model call each.")).toBeVisible();
  expect(inference(proxy)).toEqual([]);
  await user.click(discovered.getByRole("button", { name: "Start classifying new runs" }));

  await waitFor(() =>
    expect(posts(proxy, "/lens/profiles/taxonomy/activate")).toEqual([{ agent: "assistant", revision: 7, backfill: false }]),
  );
  expect(posts(proxy, "/lens/profiles/discovery/preview")).toContainEqual({
    agent: "assistant",
    start_ms: 10 * DAY,
    end_ms: 17 * DAY,
    max_runs: 500,
  });
});

it("backfills the discovery range only when the box is checked", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  serve(proxy, { taxonomy: view({ taxonomy: taxonomy() }), written: view({ active: active({ revision: 8 }) }) });
  render();

  await user.click(await screen.findByRole("checkbox", { name: /Also classify earlier runs/ }));
  await user.click(screen.getByRole("button", { name: "Start classifying new runs" }));

  await waitFor(() =>
    expect(posts(proxy, "/lens/profiles/taxonomy/activate")).toEqual([{ agent: "assistant", revision: 7, backfill: true }]),
  );
});

it("clears the backfill choice when a new discovery replaces the draft", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  const server = serve(proxy, { taxonomy: view({ taxonomy: taxonomy() }) });
  render();
  await user.click(await screen.findByRole("checkbox", { name: /Also classify earlier runs/ }));
  expect(screen.getByRole("checkbox", { name: /Also classify earlier runs/ })).toBeChecked();
  server.taxonomy = view({ taxonomy: taxonomy({ version: 3, revision: 9 }) });

  await testQueryClient.refetchQueries({ queryKey: ["lens", "profile-taxonomy"] });

  await waitFor(() => expect(screen.getByRole("checkbox", { name: /Also classify earlier runs/ })).not.toBeChecked());
});

it("says activation replaces the active categories", async () => {
  const proxy = stubGateway();
  serve(proxy, { taxonomy: view({ active: active(), taxonomy: taxonomy() }) });
  render();

  expect(await screen.findByText(/Replaces the active categories/)).toBeVisible();
  expect(screen.getByRole("region", { name: "Active categories" })).toHaveTextContent("Answer billing questions");
});

it("cancels a running rediscovery by discarding its draft without pausing the active categories", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  const running = active();
  serve(proxy, { taxonomy: view({ active: running, taxonomy: discovering() }), written: view({ active: running }) });
  render();

  expect(await screen.findByText("The active categories keep classifying meanwhile.")).toBeVisible();
  expect(screen.queryByRole("button", { name: "Rediscover categories" })).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Cancel discovery" }));

  await waitFor(() =>
    expect(posts(proxy, "/lens/profiles/taxonomy/reset")).toEqual([{ agent: "assistant", revision: 7, target: "draft" }]),
  );
  expect(proxy.put).not.toHaveBeenCalled();
  expect(await screen.findByRole("button", { name: "Rediscover categories" })).toBeVisible();
  expect(screen.getByRole("region", { name: "Active categories" })).toHaveTextContent(/Classifying new runs since/);
});

it("discards a failed discovery and shows its error", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  serve(proxy, { taxonomy: view({ taxonomy: taxonomy({ status: "failed", error: "Embedding model unavailable" }) }) });
  render();

  expect(await screen.findByText("Embedding model unavailable")).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Discard draft" }));

  await waitFor(() =>
    expect(posts(proxy, "/lens/profiles/taxonomy/reset")).toEqual([{ agent: "assistant", revision: 7, target: "draft" }]),
  );
});

it("pauses by turning discovery off and resumes by turning it on and then resuming at the current revisions", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  const server = serve(proxy, { taxonomy: view({ active: active() }), written: view({ active: active({ revision: 5 }) }) });
  proxy.put.mockImplementationOnce(async () => {
    server.taxonomy = view({ active: active({ revision: 4, windows: [{ start_ms: 20 * DAY, end_ms: 21 * DAY }] }) });
    server.settings = configured({ enabled: false, revision: 4 });
    return server.settings.settings;
  });
  render();

  await user.click(await screen.findByRole("button", { name: "Pause classification" }));

  expect(await screen.findByText(/Classification is paused/)).toBeVisible();
  expect(proxy.put).toHaveBeenCalledWith(
    "/lens/profiles/settings",
    expect.objectContaining({ body: { agent: "assistant", revision: 3, enabled: false, models } }),
  );
  await user.click(screen.getByRole("button", { name: "Resume classification" }));

  await waitFor(() => expect(posts(proxy, "/lens/profiles/taxonomy/resume")).toEqual([{ agent: "assistant", revision: 4 }]));
  expect(proxy.put).toHaveBeenLastCalledWith(
    "/lens/profiles/settings",
    expect.objectContaining({ body: { agent: "assistant", revision: 4, enabled: true, models } }),
  );
  const resumeCall = proxy.post.mock.calls.findIndex(([path]) => path === "/lens/profiles/taxonomy/resume");
  expect(proxy.put.mock.invocationCallOrder[1]).toBeLessThan(proxy.post.mock.invocationCallOrder[resumeCall]);
  expect(await screen.findByText(/Classifying new runs since/)).toBeVisible();
});

it("uses the revision a discovery returned for the next write before the view is read again", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  serve(proxy);
  const write = proxy.post.getMockImplementation();
  proxy.post.mockImplementation(async (path, request) =>
    path === "/lens/profiles/discovery" ? view({ taxonomy: discovering({ revision: 8 }) }) : write?.(path, request),
  );
  render();
  await screen.findByText("140 eligible runs. Discovery reads the most recent 140.");
  const read = proxy.get.getMockImplementation();
  proxy.get.mockImplementation(async (path, request) =>
    path === "/lens/profiles/taxonomy" ? new Promise(() => {}) : read?.(path, request),
  );

  await user.click(screen.getByRole("button", { name: "Discover categories" }));
  await user.click(screen.getByRole("button", { name: "Start discovery" }));
  await user.click(await screen.findByRole("button", { name: "Cancel discovery" }));

  await waitFor(() =>
    expect(posts(proxy, "/lens/profiles/taxonomy/reset")).toEqual([{ agent: "assistant", revision: 8, target: "draft" }]),
  );
});

it("uses the settings revision a save returned for the next write before the settings are read again", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  serve(proxy, { taxonomy: view({ active: active() }) });
  render();
  await screen.findByRole("button", { name: "Pause classification" });
  const read = proxy.get.getMockImplementation();
  proxy.get.mockImplementation(async (path, request) =>
    path === "/lens/profiles/settings" ? new Promise(() => {}) : read?.(path, request),
  );

  await choose(user, "Discovery model", "claude-sonnet");
  await user.click(screen.getByRole("button", { name: "Save models" }));
  expect(await screen.findByText("Saved")).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Pause classification" }));

  await waitFor(() => expect(proxy.put).toHaveBeenCalledTimes(2));
  expect(proxy.put.mock.calls.map(([, request]) => request.body)).toEqual([
    { agent: "assistant", revision: 3, enabled: true, models: { ...models, discovery: "claude-sonnet" } },
    { agent: "assistant", revision: 4, enabled: false, models: { ...models, discovery: "claude-sonnet" } },
  ]);
});

it("reloads settings after a rejected save so retrying uses the newer revision and keeps the edited models", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  const server = serve(proxy);
  proxy.put.mockImplementationOnce(async () => {
    server.settings = configured({ revision: 5 });
    throw new Error("Task discovery changed since it was loaded; reload and try again");
  });
  render();
  await screen.findByText("140 eligible runs. Discovery reads the most recent 140.");

  await choose(user, "Discovery model", "claude-sonnet");
  await user.click(screen.getByRole("button", { name: "Save models" }));
  expect(await screen.findByText("Task discovery changed since it was loaded; reload and try again")).toBeVisible();
  await waitFor(() =>
    expect(testQueryClient.getQueryData<ProfileSettingsView>(["lens", "profile-settings", "test", "assistant"])?.settings.revision).toBe(5),
  );
  await user.click(screen.getByRole("button", { name: "Save models" }));

  await waitFor(() => expect(proxy.put).toHaveBeenCalledTimes(2));
  expect(proxy.put.mock.calls.map(([, request]) => request.body)).toEqual([
    { agent: "assistant", revision: 3, enabled: true, models: { ...models, discovery: "claude-sonnet" } },
    { agent: "assistant", revision: 5, enabled: true, models: { ...models, discovery: "claude-sonnet" } },
  ]);
  expect(inference(proxy)).toEqual([]);
});

it("reloads the taxonomy after a rejected activation so retrying uses the newer draft revision", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  const server = serve(proxy, { taxonomy: view({ taxonomy: taxonomy() }), written: view({ active: active({ revision: 10 }) }) });
  const write = proxy.post.getMockImplementation();
  let rejected = false;
  proxy.post.mockImplementation(async (path, request) => {
    if (path !== "/lens/profiles/taxonomy/activate" || rejected) return write?.(path, request);
    rejected = true;
    server.taxonomy = view({ taxonomy: taxonomy({ version: 3, revision: 9 }) });
    throw new Error("Task discovery changed since it was loaded; reload and try again");
  });
  render();

  await user.click(await screen.findByRole("button", { name: "Start classifying new runs" }));
  expect(await screen.findByText("Task discovery changed since it was loaded; reload and try again")).toBeVisible();
  await waitFor(() =>
    expect(testQueryClient.getQueryData<TaxonomyView>(["lens", "profile-taxonomy", "test", "assistant"])?.taxonomy?.revision).toBe(9),
  );
  await user.click(screen.getByRole("button", { name: "Start classifying new runs" }));

  await waitFor(() => expect(posts(proxy, "/lens/profiles/taxonomy/activate")).toHaveLength(2));
  expect(posts(proxy, "/lens/profiles/taxonomy/activate")).toEqual([
    { agent: "assistant", revision: 7, backfill: false },
    { agent: "assistant", revision: 9, backfill: false },
  ]);
});
