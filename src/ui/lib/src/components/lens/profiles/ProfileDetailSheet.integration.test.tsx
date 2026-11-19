import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../tests/test-utils";
import { ProfileDetailSheet } from "./ProfileDetailSheet";
import type { ProfileDetail, ProfilesMatrix, TaskDefinition } from "./schemas";

const definition: TaskDefinition = {
  task_id: "charts",
  name: "Read charts",
  description: "Answer questions about an attached chart",
  inclusion_criteria: "Asks about a chart or screenshot",
  exclusion_criteria: "Plain text questions",
  required_capabilities: ["vision", "structured_output"],
  example_trace_ids: [],
};

const detail = (overrides: Partial<ProfileDetail> = {}): ProfileDetail => ({
  window: { agent: "assistant", start_ms: 0, end_ms: 1 },
  coverage: { runs_listed: 5000, runs_resolved: 1200, truncated: true },
  profile: {
    profile_id: "task:charts",
    name: "Read charts",
    source: "enhanced",
    run_count: 3,
    share: 0.5,
    request_count: 4,
    cost_observations: 2,
    spend_usd: 0.5,
  },
  definition,
  runs: [{ trace_id: "trace-quiet", trace_ref: "REF9", start_ms: 0, input_preview: "", request_count: 1, spend_usd: null, resolved: true }],
  models: [
    {
      profile_id: "task:charts",
      column_key: "id:dep-vision",
      request_count: 3,
      avg_cost_usd: 0.25,
      cost_observations: 3,
      p95_latency_ms: 1500,
      latency_observations: 3,
      eval_success_rate: null,
      eval_sample_count: 0,
    },
    {
      profile_id: "task:charts",
      column_key: "unmatched:mystery-model",
      request_count: 1,
      avg_cost_usd: null,
      cost_observations: 0,
      p95_latency_ms: null,
      latency_observations: 0,
      eval_success_rate: null,
      eval_sample_count: 0,
    },
  ],
  ...overrides,
});

const matrix: ProfilesMatrix = {
  window: { agent: "assistant", start_ms: 0, end_ms: 1 },
  coverage: { runs_listed: 3, runs_resolved: 3, truncated: false },
  profiles: [],
  models: [
    {
      column_key: "id:dep-vision",
      model_config_id: "dep-vision",
      model_name: "vision-large",
      model_group: "vision",
      provider: "openai",
      configured: true,
      capabilities: { function_calling: null, vision: null, reasoning: null, structured_output: null },
    },
  ],
  cells: [],
  compatibility: [],
  other_agent_requests: 0,
};

function render(data: unknown, options: { canInvestigate?: boolean; investigateDisabled?: boolean } = {}) {
  const proxy = stubGateway();
  proxy.get.mockImplementation(async (path) => {
    if (path === "/lens/profiles/matrix") return matrix;
    if (data instanceof Error) throw data;
    return data;
  });
  const onUrlUpdate = vi.fn();
  const onInvestigate = vi.fn();
  renderWithLens(
    <ProfileDetailSheet
      agent="assistant"
      window={{ startMs: 0, endMs: 1 }}
      profileId="task:charts"
      canInvestigate={options.canInvestigate ?? true}
      investigateDisabled={options.investigateDisabled ?? false}
      onInvestigate={onInvestigate}
      onClose={vi.fn()}
    />,
    { onUrlUpdate },
  );
  return { onUrlUpdate, onInvestigate, proxy };
}

beforeEach(() => testQueryClient.clear());
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it("shows one category description and aligned per-model figures without inventing missing values", async () => {
  render(detail());

  const sheet = within(await screen.findByRole("dialog", { name: "Read charts" }));
  expect(sheet.getByText("Answer questions about an attached chart")).toBeVisible();
  expect(sheet.queryByText(/Asks about a chart or screenshot/)).not.toBeInTheDocument();
  expect(sheet.queryByText(/Plain text questions/)).not.toBeInTheDocument();
  expect(sheet.queryByText(/Requires/)).not.toBeInTheDocument();
  const models = within(sheet.getByRole("region", { name: "Models" }));
  const named = await models.findByRole("row", { name: /vision-large/ });
  expect(named).toHaveTextContent("dep-vision");
  const cells = (row: HTMLElement) => within(row).getAllByRole("cell").map((cell) => cell.textContent);
  expect(cells(named)).toEqual(["3", "$0.25", "1.5 s"]);
  expect(cells(models.getByRole("row", { name: /mystery-model/ }))).toEqual(["1", "Unknown", "Unknown"]);
  expect(sheet.getByText("Figures cover 1,200 of at least 5,000 runs in this range.")).toBeVisible();
});

it("opens a run without a request preview by its trace id and keeps its unknown cost unknown", async () => {
  const user = userEvent.setup();
  const { onUrlUpdate } = render(detail());

  const run = await within(await screen.findByRole("region", { name: "Recent runs" })).findByRole("button", { name: /trace-quiet/ });
  expect(run).toHaveTextContent("Unknown");
  expect(run).not.toHaveTextContent("$0");
  await user.click(run);

  await waitFor(() => expect(new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString).get("trace")).toBe("trace-quiet"));
  expect(new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString).get("trace_ref")).toBe("REF9");
});

it("says how many runs the figures cover without calling a complete run list a lower bound", async () => {
  render(detail({ coverage: { runs_listed: 3, runs_resolved: 2, truncated: false } }));

  expect(await screen.findByText("Figures cover 2 of 3 runs in this range.")).toBeVisible();
});

it("says when a profile has no model requests and shows no description or requirement copy without a definition", async () => {
  render(detail({ definition: null, models: [], coverage: { runs_listed: 3, runs_resolved: 3, truncated: false } }));

  expect(await screen.findByText("No model requests by assistant in these runs.")).toBeVisible();
  expect(screen.queryByText(/requirements/i)).not.toBeInTheDocument();
  expect(screen.queryByText(/Figures cover/)).not.toBeInTheDocument();
});

it("says model figures are unavailable instead of zero when the runs were not fully read", async () => {
  render(detail({ models: [], coverage: { runs_listed: 3, runs_resolved: 1, truncated: false } }));

  expect(await screen.findByText("Model figures are unavailable for these runs.")).toBeVisible();
  expect(screen.queryByText(/No model requests/)).not.toBeInTheDocument();
});

it("shows an unresolved run's requests and spend as unavailable rather than zero", async () => {
  const unread = { trace_id: "trace-unread", trace_ref: "REF2", start_ms: 0, input_preview: "Plot sales", request_count: 0, spend_usd: 0, resolved: false };
  render(detail({ runs: [unread] }));

  const run = await within(await screen.findByRole("region", { name: "Recent runs" })).findByRole("button", { name: /Plot sales/ });
  expect(run).toHaveTextContent(/Unavailable.*Unavailable/);
  expect(run).not.toHaveTextContent("$0");
});

it("treats a run from an older response without a resolved flag as resolved", async () => {
  const older = { trace_id: "trace-quiet", trace_ref: "REF9", start_ms: 0, input_preview: "", request_count: 0, spend_usd: 0 };
  render({ ...detail(), runs: [older] });

  const run = await within(await screen.findByRole("region", { name: "Recent runs" })).findByRole("button", { name: /trace-quiet/ });
  expect(run).toHaveTextContent("$0.00");
  expect(run).not.toHaveTextContent("Unavailable");
});

it("hides investigation from callers who cannot investigate", async () => {
  render(detail(), { canInvestigate: false });
  await screen.findByRole("dialog", { name: "Read charts" });

  expect(screen.queryByRole("button", { name: "Investigate these runs" })).not.toBeInTheDocument();
});

it("keeps investigation disabled for a zoomed range", async () => {
  render(detail(), { investigateDisabled: true });

  expect(await screen.findByRole("button", { name: "Investigate these runs" })).toBeDisabled();
});

it("investigates the profile shown", async () => {
  const user = userEvent.setup();
  const { onInvestigate } = render(detail());

  await user.click(await screen.findByRole("button", { name: "Investigate these runs" }));

  expect(onInvestigate).toHaveBeenCalledWith("task:charts");
});

it("stops showing earlier figures and runs when a refetch of the detail fails", async () => {
  const { proxy } = render(detail());
  expect(await screen.findByRole("button", { name: /trace-quiet/ })).toBeVisible();
  proxy.get.mockRejectedValue(new Error("Profiles unavailable"));

  await testQueryClient.refetchQueries({ queryKey: ["lens", "profile-detail"] });

  expect(await screen.findByRole("alert")).toHaveTextContent("Profiles unavailable");
  expect(screen.queryByRole("button", { name: /trace-quiet/ })).not.toBeInTheDocument();
  expect(screen.queryByText("Known spend")).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Investigate profile" })).not.toBeInTheDocument();
});

it("reports a failed detail load", async () => {
  render(new Error("Profile not found in this agent and time range"));

  expect(await screen.findByRole("alert")).toHaveTextContent("Profile not found in this agent and time range");
});
