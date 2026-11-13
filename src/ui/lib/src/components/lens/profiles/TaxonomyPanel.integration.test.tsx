import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../tests/test-utils";
import type { Taxonomy, TaxonomyView } from "./schemas";
import { TaxonomyPanel } from "./TaxonomyPanel";

const DAY = 86_400_000;

const taxonomy = (overrides: Partial<Taxonomy>): Taxonomy => ({
  taxonomy_id: "tax",
  agent: "assistant",
  version: 2,
  revision: 7,
  status: "draft",
  sample: false,
  tasks: [
    {
      task_id: "debug",
      name: "Debug failing tests",
      description: "Fix a failing test",
      inclusion_criteria: "Asks to fix a test",
      exclusion_criteria: "New features",
      required_capabilities: ["function_calling"],
      example_trace_ids: [],
    },
  ],
  points: [],
  job: null,
  discovered_at: null,
  frozen_at: null,
  error: null,
  models: { distill: "haiku", embedding: "qwen", synthesis: "haiku", decision: "jev" },
  discovery: { start_ms: 10 * DAY, end_ms: 17 * DAY, requested_runs: 300, sampled_runs: 120 },
  windows: [],
  ...overrides,
});

const view = (overrides: Partial<TaxonomyView>): TaxonomyView => ({
  taxonomy: null,
  active: null,
  discovery_available: true,
  can_write: true,
  ...overrides,
});

const render = () => renderWithLens(<TaxonomyPanel agent="assistant" onConfigure={vi.fn()} />);

beforeEach(() => testQueryClient.clear());
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it("lets an admin correct a draft's definitions and requirements before activation", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  proxy.get.mockResolvedValue(view({ taxonomy: taxonomy({}) }));
  proxy.post.mockResolvedValue({ eligible_runs: 140, sample_runs: 140, previews: [] });
  proxy.patch.mockResolvedValue(view({ taxonomy: taxonomy({ revision: 8 }) }));
  render();

  expect(await screen.findByDisplayValue("Asks to fix a test")).toBeVisible();
  expect(screen.getByRole("checkbox", { name: "Function calling" })).toBeChecked();
  await user.click(screen.getByRole("checkbox", { name: "Vision" }));
  await user.click(screen.getByRole("button", { name: "Save task" }));

  await waitFor(() =>
    expect(proxy.patch).toHaveBeenCalledWith(
      "/lens/profiles/taxonomy/tasks/debug",
      expect.objectContaining({
        body: expect.objectContaining({ revision: 7, required_capabilities: ["function_calling", "vision"] }),
      }),
    ),
  );
});

it("activates for new runs only unless the counted backfill is chosen", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  proxy.get.mockResolvedValue(view({ taxonomy: taxonomy({}) }));
  proxy.post.mockImplementation(async (path) => {
    if (path === "/lens/profiles/discovery/preview") return { eligible_runs: 140, sample_runs: 140, previews: [] };
    return view({});
  });
  render();

  const backfill = await screen.findByRole("checkbox", { name: /Also classify earlier runs/ });
  expect(backfill).not.toBeChecked();
  expect(await screen.findByText("At least 140 runs, one paid Jev call each.")).toBeVisible();
  expect(screen.getByText(/one paid jev call for each new run/i)).toBeVisible();
  await user.click(backfill);
  await user.click(screen.getByRole("button", { name: "Activate and classify new runs" }));

  await waitFor(() =>
    expect(proxy.post).toHaveBeenCalledWith(
      "/lens/profiles/taxonomy/activate",
      expect.objectContaining({ body: { agent: "assistant", revision: 7, backfill: true } }),
    ),
  );
  expect(proxy.post).toHaveBeenCalledWith(
    "/lens/profiles/discovery/preview",
    expect.objectContaining({ body: { agent: "assistant", start_ms: 10 * DAY, end_ms: 17 * DAY, max_runs: 500 } }),
  );
});

it("keeps the active tasks visible while a rediscovery runs and cancels only the draft", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  const active = taxonomy({
    status: "frozen",
    revision: 3,
    tasks: [{ ...taxonomy({}).tasks[0], task_id: "billing", name: "Answer billing questions" }],
    windows: [{ start_ms: 20 * DAY, end_ms: null }],
  });
  proxy.get.mockResolvedValue(
    view({
      active,
      taxonomy: taxonomy({
        status: "discovering",
        tasks: [],
        discovery: null,
        job: { job_id: "job", requested_at: "", lease_until: null, attempts: 1, max_runs: 300, start_ms: 0, end_ms: DAY },
      }),
    }),
  );
  proxy.post.mockResolvedValue(view({ active }));
  render();

  expect(await screen.findByText("Answer billing questions")).toBeVisible();
  expect(screen.getByText(/The active tasks keep classifying until you activate the new draft/)).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Cancel discovery" }));

  await waitFor(() =>
    expect(proxy.post).toHaveBeenCalledWith(
      "/lens/profiles/taxonomy/reset",
      expect.objectContaining({ body: { agent: "assistant", revision: 7, target: "draft" } }),
    ),
  );
});

it("offers to resume paused classification only while discovery is on", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  const paused = taxonomy({ status: "frozen", revision: 4, windows: [{ start_ms: 0, end_ms: DAY }] });
  proxy.get.mockResolvedValueOnce(view({ active: paused, discovery_available: false }));
  const { unmount } = render();

  expect(await screen.findByRole("button", { name: "Resume classification for new runs" })).toBeDisabled();
  expect(screen.getByText(/Classification is paused/)).toBeVisible();
  unmount();
  testQueryClient.clear();

  proxy.get.mockResolvedValue(view({ active: paused }));
  proxy.post.mockResolvedValue(view({ active: paused }));
  render();
  await user.click(await screen.findByRole("button", { name: "Resume classification for new runs" }));

  await waitFor(() =>
    expect(proxy.post).toHaveBeenCalledWith(
      "/lens/profiles/taxonomy/resume",
      expect.objectContaining({ body: { agent: "assistant", revision: 4 } }),
    ),
  );
});

it("keeps sample activation free of model calls and never counts a backfill for it", async () => {
  const proxy = stubGateway();
  proxy.get.mockResolvedValue(view({ taxonomy: taxonomy({ sample: true }), discovery_available: false }));
  render();

  const section = within(await screen.findByRole("region", { name: "Reviewed tasks" }));
  expect(section.getByRole("button", { name: "Activate sample (no model calls)" })).toBeEnabled();
  expect(section.getByText(/makes no model calls/)).toBeVisible();
  expect(section.queryByRole("checkbox", { name: /Also classify earlier runs/ })).not.toBeInTheDocument();
  expect(proxy.post).not.toHaveBeenCalled();
});
