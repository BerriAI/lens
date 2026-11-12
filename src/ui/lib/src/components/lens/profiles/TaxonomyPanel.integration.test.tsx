import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../tests/test-utils";
import type { Taxonomy } from "./schemas";
import { TaxonomyPanel } from "./TaxonomyPanel";

const window = { startMs: 0, endMs: 86_400_000 };

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
  models: { distill: "haiku", embedding: "qwen", synthesis: "sonnet", decision: "jev" },
  ...overrides,
});

beforeEach(() => testQueryClient.clear());
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it("lets an admin review and correct what Jev will bucket against before freezing", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  proxy.get.mockResolvedValue({ taxonomy: taxonomy({}), discovery_available: true, can_write: true });
  proxy.patch.mockResolvedValue(taxonomy({ revision: 8 }));
  renderWithLens(<TaxonomyPanel agent="assistant" window={window} />);

  expect(await screen.findByDisplayValue("Asks to fix a test")).toBeVisible();
  expect(screen.getByRole("checkbox", { name: "Function calling" })).toBeChecked();
  expect(screen.getByText(/Freezing starts ongoing Jev classification calls/)).toBeVisible();
  await user.click(screen.getByRole("checkbox", { name: "Vision" }));
  await user.click(screen.getByRole("button", { name: "Save task" }));

  await waitFor(() => expect(proxy.patch).toHaveBeenCalled());
  expect(proxy.patch).toHaveBeenCalledWith(
    "/lens/profiles/taxonomy/tasks/debug",
    expect.objectContaining({
      body: expect.objectContaining({ revision: 7, required_capabilities: ["function_calling", "vision"] }),
    }),
  );
});

it("offers to cancel a running discovery with the current revision", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  proxy.get.mockResolvedValue({ taxonomy: taxonomy({ status: "discovering", tasks: [] }), discovery_available: true, can_write: true });
  proxy.post.mockResolvedValue(taxonomy({ status: "cleared", revision: 8 }));
  renderWithLens(<TaxonomyPanel agent="assistant" window={window} />);

  await user.click(await screen.findByRole("button", { name: "Cancel discovery" }));

  await waitFor(() =>
    expect(proxy.post).toHaveBeenCalledWith(
      "/lens/profiles/taxonomy/reset",
      expect.objectContaining({ body: { agent: "assistant", revision: 7 } }),
    ),
  );
});

it("labels a sample freeze as making no model calls", async () => {
  const proxy = stubGateway();
  proxy.get.mockResolvedValue({ taxonomy: taxonomy({ sample: true }), discovery_available: false, can_write: true });
  renderWithLens(<TaxonomyPanel agent="assistant" window={window} />);

  expect(await screen.findByRole("button", { name: "Freeze sample (no model calls)" })).toBeVisible();
  expect(screen.queryByText(/Freezing starts ongoing Jev classification calls/)).not.toBeInTheDocument();
});
