import { describe, expect, it } from "vitest";
import { createLensDemo } from "./createLensDemo";
import { demoProfilesApi } from "./demoProfiles";
import { createLensDemoData } from "./fixtures";

const NOW = Date.UTC(2026, 10, 13, 12);
const everything = { startMs: 0, endMs: NOW + 1 };
const profiles = () => createLensDemo(NOW).profiles;

describe("demo profiles", () => {
  it("builds a matrix whose rows are the overview's profiles and whose cells price only priced runs", async () => {
    const api = profiles();
    const overview = await api.overview(everything, "support_agent");
    const matrix = await api.matrix(everything, "support_agent");

    expect(matrix.profiles.map((profile) => profile.profile_id)).toEqual(overview.profiles.map((profile) => profile.profile_id));
    expect(matrix.cells.length).toBeGreaterThan(0);
    expect(matrix.cells.every((cell) => (cell.avg_cost_usd === null) === (cell.cost_observations === 0))).toBe(true);
    expect(new Set(matrix.models.map((model) => model.column_key))).toEqual(new Set(matrix.cells.map((cell) => cell.column_key)));
  });

  it("attributes each model call once and counts cost observations per request, not per run", async () => {
    const data = createLensDemoData(NOW);
    const [first, ...rest] = data.runs;
    const llm = first.trace.spans.find((span) => span.type === "llm");
    if (!llm) throw new Error("demo run has no model call");
    const extra = [
      { ...llm, span_id: "extra-priced", model: "demo-small-model", spend: 0.01 },
      { ...llm, span_id: "extra-unpriced", model: "demo-small-model", spend: null },
    ];
    const multi = { ...first, trace: { ...first.trace, spans: [...first.trace.spans, ...extra] } };
    const api = demoProfilesApi({ ...data, runs: [multi, ...rest] }, async () => []);
    const agent = first.trace.summary.agent_names[0];

    const overview = await api.overview(everything, agent);
    const matrix = await api.matrix(everything, agent);
    const calls = [multi, ...rest]
      .filter((run) => run.trace.summary.agent_names.includes(agent))
      .flatMap((run) => run.trace.spans.filter((span) => span.type === "llm"));
    const pricedCalls = calls.filter((call) => call.spend !== null);
    const sum = (values: number[]) => values.reduce((total, value) => total + value, 0);

    expect(overview.totals.requests).toBe(calls.length);
    expect(overview.totals.cost_observations).toBe(pricedCalls.length);
    expect(sum(overview.profiles.map((profile) => profile.cost_observations))).toBe(pricedCalls.length);
    expect(sum(matrix.cells.map((cell) => cell.request_count))).toBe(calls.length);
    expect(sum(matrix.cells.map((cell) => (cell.avg_cost_usd ?? 0) * cell.cost_observations))).toBeCloseTo(
      sum(pricedCalls.map((call) => call.spend ?? 0)),
      10,
    );
    expect(overview.totals.spend_usd).toBeCloseTo(sum(pricedCalls.map((call) => call.spend ?? 0)), 10);
  });

  it("opens a profile by id or by its older label link", async () => {
    const api = profiles();
    const [first] = (await api.overview(everything, "support_agent")).profiles;

    const byId = await api.detail(everything, "support_agent", first.profile_id);
    const byName = await api.detail(everything, "support_agent", first.name);

    expect(byName).toEqual(byId);
    expect(byId.runs).toHaveLength(first.run_count);
    expect(byId.definition).toBeNull();
  });

  it("rejects a profile the agent does not have", async () => {
    await expect(profiles().detail(everything, "support_agent", "recorded:Missing")).rejects.toThrow(
      "Profile not found in this agent and time range",
    );
  });
});
