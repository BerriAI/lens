import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../tests/test-utils";
import { ProfileDetailSheet } from "./ProfileDetailSheet";
import type { ProfileDetail } from "./schemas";

const detail = (overrides: Partial<ProfileDetail> = {}): ProfileDetail => ({
  window: { agent: "assistant", start_ms: 0, end_ms: 1 },
  coverage: { runs_listed: 5000, runs_resolved: 1200, truncated: true },
  profile: {
    profile_id: "recorded:Charts",
    name: "Read charts",
    source: "recorded",
    run_count: 3,
    share: 0.5,
    request_count: 4,
    cost_observations: 2,
    spend_usd: 0.5,
  },
  runs: [{ trace_id: "trace-quiet", trace_ref: "REF9", start_ms: 0, input_preview: "", request_count: 1, spend_usd: null }],
  models: [
    {
      profile_id: "recorded:Charts",
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
      profile_id: "recorded:Charts",
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

function render(data: ProfileDetail | Error, options: { canInvestigate?: boolean; investigateDisabled?: boolean } = {}) {
  const proxy = stubGateway();
  proxy.get.mockImplementation(async () => {
    if (data instanceof Error) throw data;
    return data;
  });
  const onUrlUpdate = vi.fn();
  const onInvestigate = vi.fn();
  renderWithLens(
    <ProfileDetailSheet
      agent="assistant"
      window={{ startMs: 0, endMs: 1 }}
      profileId="recorded:Charts"
      canInvestigate={options.canInvestigate ?? true}
      investigateDisabled={options.investigateDisabled ?? false}
      onInvestigate={onInvestigate}
      onClose={vi.fn()}
    />,
    { onUrlUpdate },
  );
  return { onUrlUpdate, onInvestigate };
}

beforeEach(() => testQueryClient.clear());
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it("shows per-model figures and partial coverage without inventing missing values", async () => {
  render(detail());

  const sheet = within(await screen.findByRole("dialog", { name: "Read charts" }));
  expect(sheet.getByText("dep-vision")).toBeVisible();
  expect(sheet.getByText("3 requests · $0.25 avg · 1.5 s p95")).toBeVisible();
  expect(sheet.getByText("mystery-model")).toBeVisible();
  expect(sheet.getByText("1 requests · cost unknown avg · latency unknown p95")).toBeVisible();
  expect(sheet.getByText("Figures cover 1,200 of at least 5,000 runs in this range.")).toBeVisible();
});

it("opens a run without a request preview by its trace id", async () => {
  const user = userEvent.setup();
  const { onUrlUpdate } = render(detail());

  const run = await screen.findByRole("button", { name: /trace-quiet/ });
  expect(run).toHaveTextContent("spend unknown");
  await user.click(run);

  await waitFor(() => expect(new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString).get("trace")).toBe("trace-quiet"));
  expect(new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString).get("trace_ref")).toBe("REF9");
});

it("says when a profile has no model requests", async () => {
  render(detail({ models: [], coverage: { runs_listed: 3, runs_resolved: 3, truncated: false } }));

  expect(await screen.findByText("No model requests by assistant in these runs.")).toBeVisible();
  expect(screen.queryByText(/Figures cover/)).not.toBeInTheDocument();
});

it("hides investigation from callers who cannot investigate", async () => {
  render(detail(), { canInvestigate: false });
  await screen.findByRole("dialog", { name: "Read charts" });

  expect(screen.queryByRole("button", { name: "Investigate profile" })).not.toBeInTheDocument();
});

it("keeps investigation disabled for a zoomed range", async () => {
  render(detail(), { investigateDisabled: true });

  expect(await screen.findByRole("button", { name: "Investigate profile" })).toBeDisabled();
});

it("investigates the profile shown", async () => {
  const user = userEvent.setup();
  const { onInvestigate } = render(detail());

  await user.click(await screen.findByRole("button", { name: "Investigate profile" }));

  expect(onInvestigate).toHaveBeenCalledWith("recorded:Charts");
});

it("reports a failed detail load", async () => {
  render(new Error("Profile not found in this agent and time range"));

  expect(await screen.findByRole("alert")).toHaveTextContent("Profile not found in this agent and time range");
});
