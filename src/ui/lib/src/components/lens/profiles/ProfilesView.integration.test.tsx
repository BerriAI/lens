import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../tests/test-utils";
import type { LensAgents } from "../agents/AgentScoped";
import { ProfilesView } from "./ProfilesView";

const ranked = async (by = "runs") => within(await screen.findByRole("list", { name: `Profiles ranked by ${by}` }));
import type { ProfileDetail, ProfilesMatrix, ProfilesOverview, ProfileSummary } from "./schemas";

const agents: LensAgents = {
  agent: "assistant",
  select: vi.fn(),
  list: {
    agents: [{ name: "assistant", runs: 20, failed_runs: 0, last_seen: new Date().toISOString(), frameworks: [] }],
    isLoading: false,
    error: null,
  },
};

const summary = (profile_id: string, name: string, overrides: Partial<ProfileSummary> = {}): ProfileSummary => ({
  profile_id,
  name,
  source: profile_id.startsWith("recorded:") ? "recorded" : "unclassified",
  run_count: 10,
  share: 0.5,
  request_count: 12,
  cost_observations: 12,
  spend_usd: 1.5,
  ...overrides,
});

const overview = (overrides: Partial<ProfilesOverview> = {}): ProfilesOverview => ({
  window: { agent: "assistant", start_ms: 0, end_ms: 1 },
  coverage: { runs_listed: 20, runs_resolved: 20, truncated: false },
  totals: {
    runs: 20,
    requests: 18,
    matched_requests: 15,
    cost_observations: 12,
    spend_usd: 1.5,
    other_agent_requests: 0,
    shared_requests: 0,
  },
  profiles: [
    summary("recorded:Coding", "Coding"),
    summary("unclassified", "Unclassified", { spend_usd: null, cost_observations: 0, request_count: 6 }),
  ],
  ...overrides,
});

const matrix: ProfilesMatrix = {
  window: { agent: "assistant", start_ms: 0, end_ms: 1 },
  coverage: { runs_listed: 20, runs_resolved: 20, truncated: false },
  profiles: [{ profile_id: "recorded:Coding", name: "Coding", source: "recorded" }],
  models: [
    {
      column_key: "id:dep-sonnet",
      model_config_id: "dep-sonnet",
      model_name: "claude-sonnet",
      model_group: "claude-sonnet",
      provider: "anthropic",
      configured: true,
      capabilities: { function_calling: true, vision: null, reasoning: null, structured_output: null },
    },
  ],
  cells: [
    {
      profile_id: "recorded:Coding",
      column_key: "id:dep-sonnet",
      request_count: 12,
      avg_cost_usd: null,
      cost_observations: 0,
      p95_latency_ms: 900,
      latency_observations: 12,
      eval_success_rate: null,
      eval_sample_count: 0,
    },
  ],
  other_agent_requests: 0,
};

const detail = (profileId: string): ProfileDetail => ({
  window: { agent: "assistant", start_ms: 0, end_ms: 1 },
  coverage: { runs_listed: 20, runs_resolved: 20, truncated: false },
  profile: summary(profileId, "Coding"),
  runs: [{ trace_id: "trace-1", trace_ref: "REF1", start_ms: 0, input_preview: "Fix the login test", request_count: 2, spend_usd: 0.4 }],
  models: [],
});

function routes(proxy: ReturnType<typeof stubGateway>, data: { overview?: ProfilesOverview } = {}) {
  proxy.get.mockImplementation(async (path, request) => {
    if (path === "/lens/profiles/overview") return data.overview ?? overview();
    if (path === "/lens/profiles/matrix") return matrix;
    if (path === "/lens/profiles/detail") return detail(String(request.query.profile_id));
    throw new Error(`unexpected ${path}`);
  });
}

beforeEach(() => testQueryClient.clear());
afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it("waits for the navigation agent instead of querying all agents", async () => {
  const proxy = stubGateway();
  renderWithLens(<ProfilesView agents={{ ...agents, agent: null }} />);
  expect(await screen.findByText("Select an agent to explore its tasks")).toBeVisible();
  expect(proxy.get).not.toHaveBeenCalled();
});

it("shows run and spend figures, never a missing cost as zero, and selects a profile by id", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  routes(proxy);
  const onUrlUpdate = vi.fn();
  renderWithLens(<ProfilesView agents={agents} />, { searchParams: "?tab=profiles&agent=assistant", onUrlUpdate });

  const list = await ranked();
  const unclassified = list.getByRole("button", { name: /Unclassified/ });
  expect(unclassified).toHaveTextContent("no cost data");
  expect(unclassified).not.toHaveTextContent("$0");
  expect(screen.getByText("cost known for 12 of 18 requests")).toBeVisible();
  expect(proxy.get).toHaveBeenCalledWith("/lens/profiles/overview", expect.objectContaining({ query: expect.objectContaining({ agent: "assistant" }) }));

  await user.click(list.getByRole("button", { name: /Coding/ }));

  await waitFor(() => expect(new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString).get("profile")).toBe("recorded:Coding"));
});

it("shows heatmap cells without cost evidence as no data", async () => {
  const proxy = stubGateway();
  routes(proxy);
  renderWithLens(<ProfilesView agents={agents} />, { searchParams: "?tab=profiles&agent=assistant" });

  await userEvent.setup().click(await screen.findByRole("radio", { name: "Avg cost" }));

  expect(screen.getByRole("button", { name: "Coding on claude-sonnet: No data" })).toHaveTextContent("No data");
});

it("translates an older label link to the profile id and investigates it", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  routes(proxy);
  const onUrlUpdate = vi.fn();
  renderWithLens(<ProfilesView agents={agents} />, {
    searchParams: "?tab=profiles&agent=assistant&hours=168&profile=Coding",
    onUrlUpdate,
  });

  expect(await screen.findByText("Fix the login test")).toBeVisible();
  expect(proxy.get).toHaveBeenCalledWith(
    "/lens/profiles/detail",
    expect.objectContaining({ query: expect.objectContaining({ profile_id: "recorded:Coding" }) }),
  );
  await user.click(screen.getByRole("button", { name: "Investigate profile" }));

  await waitFor(() => expect(onUrlUpdate.mock.lastCall?.[0].queryString).toContain("dialog=new"));
  const url = new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString);
  expect(url.get("profile")).toBe("recorded:Coding");
  expect(url.get("agent")).toBe("assistant");
  expect(url.get("hours")).toBe("168");
  expect(url.get("tab")).toBe("findings");
});

it("says when request figures cover only part of the listed runs", async () => {
  const proxy = stubGateway();
  routes(proxy, { overview: overview({ coverage: { runs_listed: 5000, runs_resolved: 1200, truncated: true } }) });
  renderWithLens(<ProfilesView agents={agents} />, { searchParams: "?tab=profiles&agent=assistant" });

  expect(await screen.findByText(/cover 1,200 of 5,000 listed runs/)).toBeVisible();
});

it("lets a failed overview be retried without showing stale figures", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  routes(proxy);
  const serve = proxy.get.getMockImplementation();
  let failed = false;
  proxy.get.mockImplementation(async (path, request) => {
    if (path === "/lens/profiles/overview" && !failed) {
      failed = true;
      throw new Error("Profiles unavailable");
    }
    return serve?.(path, request);
  });
  renderWithLens(<ProfilesView agents={agents} />, { searchParams: "?tab=profiles&agent=assistant" });

  expect(await screen.findByRole("alert")).toHaveTextContent("Could not load usage profiles");
  expect(screen.queryByRole("list", { name: "Profiles ranked by runs" })).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Refresh usage profiles" }));

  expect((await ranked()).getByRole("button", { name: /Coding/ })).toBeVisible();
});

it("ranks by spend with unknown cost last instead of as zero", async () => {
  const user = userEvent.setup();
  const proxy = stubGateway();
  routes(proxy, {
    overview: overview({
      profiles: [
        summary("unclassified", "Unclassified", { spend_usd: null, cost_observations: 0, request_count: 6 }),
        summary("recorded:Free", "Free tier", { spend_usd: 0 }),
        summary("recorded:Coding", "Coding", { spend_usd: 1.5, cost_observations: 4, request_count: 12 }),
      ],
    }),
  });
  renderWithLens(<ProfilesView agents={agents} />, { searchParams: "?tab=profiles&agent=assistant" });

  await user.click(await screen.findByRole("radio", { name: "Spend" }));

  const rows = (await ranked("spend")).getAllByRole("button");
  expect(rows.map((row) => row.textContent)).toEqual([
    expect.stringContaining("Coding"),
    expect.stringContaining("Free tier"),
    expect.stringContaining("Unclassified"),
  ]);
  expect(rows[0]).toHaveTextContent("cost known for 4 of 12 requests");
  expect(rows[1]).toHaveTextContent("$0.00");
  expect(rows[2]).toHaveTextContent("No cost data");
});
