import { describe, expect, it } from "vitest";
import {
  cellReading,
  formatLatency,
  formatUsd,
  heatIntensity,
  modelLabel,
  NEUTRAL,
  profileColors,
  rankProfiles,
  selectedProfileId,
  similarity,
} from "./profilesData";
import type { MatrixCell, ProfilesOverview, ProfileSummary } from "./schemas";

const cell = (overrides: Partial<MatrixCell>): MatrixCell => ({
  profile_id: "p",
  column_key: "id:dep",
  request_count: 4,
  avg_cost_usd: null,
  cost_observations: 0,
  p95_latency_ms: null,
  latency_observations: 0,
  eval_success_rate: null,
  eval_sample_count: 0,
  ...overrides,
});

describe("selectedProfileId", () => {
  const profiles = [
    { profile_id: "recorded:Coding", name: "Coding" },
    { profile_id: "cluster:abc", name: "Debug failing tests" },
  ];

  it("accepts a profile id", () => {
    expect(selectedProfileId(profiles, "cluster:abc")).toBe("cluster:abc");
  });

  it("translates an older label link to the profile id", () => {
    expect(selectedProfileId(profiles, "Coding")).toBe("recorded:Coding");
  });

  it("ignores a label this agent does not have", () => {
    expect(selectedProfileId(profiles, "Workflows")).toBeNull();
  });
});

const profile = (profile_id: string, overrides: Partial<ProfileSummary> = {}): ProfileSummary => ({
  profile_id,
  name: profile_id,
  source: "local",
  run_count: 1,
  share: 0,
  request_count: 1,
  cost_observations: 1,
  spend_usd: 1,
  ...overrides,
});

describe("profileColors", () => {
  const profiles = [
    profile("uncertain", { source: "uncertain", run_count: 9 }),
    profile("pending", { source: "pending", run_count: 7 }),
    profile("task:a", { source: "enhanced", run_count: 6 }),
    profile("unknown", { source: "unknown", run_count: 6 }),
    profile("cluster:a", { run_count: 3 }),
    profile("unclassified", { source: "unclassified", run_count: 8 }),
    profile("recorded:Billing", { source: "recorded", run_count: 5 }),
    profile("cluster:b", { run_count: 3 }),
  ];

  it("keeps placeholder profiles neutral and colors every real group from the chart tokens", () => {
    const colors = profileColors(profiles);

    expect(colors.get("uncertain")).toBe(NEUTRAL);
    expect(colors.get("unclassified")).toBe(NEUTRAL);
    expect(colors.get("pending")).toBe(NEUTRAL);
    expect(colors.get("unknown")).toBe(NEUTRAL);
    expect(["recorded:Billing", "task:a", "cluster:a", "cluster:b"].map((id) => colors.get(id))).toEqual([
      expect.stringContaining("var(--chart-"),
      expect.stringContaining("var(--chart-"),
      expect.stringContaining("var(--chart-"),
      expect.stringContaining("var(--chart-"),
    ]);
  });

  it("keeps a profile's color when order, traffic or the other profiles change", () => {
    const before = profileColors(profiles).get("cluster:a");
    const after = profileColors([profile("cluster:z", { run_count: 50 }), profile("cluster:a", { run_count: 1 })]).get("cluster:a");

    expect(after).toBe(before);
    expect(profileColors([...profiles].reverse())).toEqual(profileColors(profiles));
  });

  it("spreads many groups across the palette", () => {
    const colors = profileColors(Array.from({ length: 40 }, (_, index) => profile(`cluster:${index}`)));

    expect(new Set(colors.values()).size).toBeGreaterThan(5);
  });
});

describe("rankProfiles", () => {
  it("ranks by the chosen figure, breaks ties by name and scales bars to the largest", () => {
    const ranked = rankProfiles(
      [profile("b", { run_count: 2 }), profile("c", { run_count: 4 }), profile("a", { run_count: 2 })],
      "runs",
    );

    expect(ranked.map((entry) => [entry.profile.name, entry.fraction])).toEqual([
      ["c", 1],
      ["a", 0.5],
      ["b", 0.5],
    ]);
  });

  it("puts unknown spend last with an empty bar instead of ranking it as zero", () => {
    const ranked = rankProfiles(
      [profile("missing", { spend_usd: null }), profile("free", { spend_usd: 0 }), profile("paid", { spend_usd: 2 })],
      "spend",
    );

    expect(ranked.map((entry) => [entry.profile.name, entry.value, entry.fraction])).toEqual([
      ["paid", 2, 1],
      ["free", 0, 0],
      ["missing", null, 0],
    ]);
  });
});

describe("similarity", () => {
  const overview = (points: ProfilesOverview["points"], profiles: ProfileSummary[]): ProfilesOverview => ({
    window: { agent: "a", start_ms: 0, end_ms: 1 },
    coverage: { runs_listed: 7, runs_resolved: 7, truncated: false },
    totals: { runs: 7, requests: 7, matched_requests: 7, cost_observations: 7, spend_usd: 1, other_agent_requests: 0, shared_requests: 0 },
    profiles,
    points,
  });

  it("draws only positions of real groups and lists every category once with its coverage", () => {
    const data = similarity(
      overview(
        [
          { trace_id: "t1", profile_id: "task:a", x: 1, y: 1 },
          { trace_id: "t3", profile_id: "unknown", x: 3, y: 3 },
        ],
        [
          profile("task:a", { run_count: 3 }),
          profile("unknown", { source: "unknown", run_count: 2 }),
          profile("recorded:Billing", { source: "recorded", run_count: 2 }),
        ],
      ),
    );

    expect(data.groups.map((group) => [group.profile.profile_id, group.points.length])).toEqual([["task:a", 1]]);
    expect(data.coverage.map((entry) => [entry.profile.profile_id, entry.runs, entry.shown, entry.mappable])).toEqual([
      ["task:a", 3, 1, true],
      ["recorded:Billing", 2, 0, true],
      ["unknown", 2, 0, false],
    ]);
    expect(data.placed).toBe(data.coverage.reduce((sum, entry) => sum + entry.shown, 0));
  });

  it("counts scoped runs that share a trace id, profile and position as separate runs", () => {
    const point = { trace_id: "t1", profile_id: "task:a", x: 1, y: 1 };
    const data = similarity(overview([point, { ...point }], [profile("task:a", { run_count: 2 })]));

    expect(data.placed).toBe(2);
    expect(data.coverage[0]).toEqual(expect.objectContaining({ runs: 2, shown: 2 }));
    expect(new Set(data.groups[0].points.map((entry) => entry.key)).size).toBe(2);
  });

  it("keys runs that share a trace id in different teams apart", () => {
    const data = similarity(
      overview(
        [
          { trace_id: "same", profile_id: "task:a", x: 1, y: 1 },
          { trace_id: "same", profile_id: "task:a", x: 2, y: 2 },
        ],
        [profile("task:a", { run_count: 2 })],
      ),
    );

    expect(new Set(data.groups[0].points.map((point) => point.key)).size).toBe(2);
  });
});

describe("cellReading", () => {
  it("reports missing cost as no value with its observation count", () => {
    expect(cellReading(cell({ avg_cost_usd: null, cost_observations: 0 }), "cost")).toEqual({ value: null, observations: 0 });
  });

  it("returns no evidence for a model that never served the profile", () => {
    expect(cellReading(undefined, "requests")).toEqual({ value: null, observations: 0 });
  });
});

describe("formatting", () => {
  it("never shows a missing cost or latency as zero", () => {
    expect(formatUsd(null)).toBeNull();
    expect(formatLatency(null)).toBeNull();
    expect(formatUsd(0)).toBe("$0.00");
  });

  it("keeps sub-cent costs readable", () => {
    expect(formatUsd(0.0042)).toBe("$0.0042");
  });

  it("switches latency to seconds above one second", () => {
    expect(formatLatency(1500)).toBe("1.5 s");
    expect(formatLatency(320.4)).toBe("320 ms");
  });
});

describe("heatIntensity", () => {
  it("spans the observed range and leaves missing cells unshaded", () => {
    const values = [2, null, 6];

    expect(heatIntensity(2, values)).toBe(0);
    expect(heatIntensity(6, values)).toBe(1);
    expect(heatIntensity(null, values)).toBeNull();
  });
});

describe("modelLabel", () => {
  const deployment = {
    column_key: "id:dep-1",
    model_config_id: "dep-1",
    model_name: "chat-large",
    model_group: "chat",
    provider: "openai",
    configured: true,
    capabilities: { function_calling: null, vision: null, reasoning: null, structured_output: null },
  };

  it("names a known deployment by its model and keeps the deployment id as detail", () => {
    expect(modelLabel("id:dep-1", [deployment])).toEqual({ name: "chat-large", detail: "dep-1" });
  });

  it("falls back to the group when the model name is missing", () => {
    expect(modelLabel("id:dep-1", [{ ...deployment, model_name: "" }])).toEqual({ name: "chat", detail: "dep-1" });
  });

  it("strips the column prefix for a model the matrix does not list", () => {
    expect(modelLabel("unmatched:mystery", [deployment])).toEqual({ name: "mystery", detail: null });
  });
});
