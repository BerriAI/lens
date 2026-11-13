import { traceAgentNames } from "../../traces/utils";
import type { ProfilesApi } from "../../profiles/api";
import type { MatrixCell, ProfileSummary, ProfilesOverview } from "../../profiles/schemas";
import { usageCategory } from "../../profiles/usageProfile";
import type { TimeWindow } from "../../../shared/timeRange/timeRange";
import type { LensDemoData } from "./fixtures";

const readOnly = () => Promise.reject(new Error("The demo is read-only"));

interface DemoRun {
  traceId: string;
  startMs: number;
  preview: string;
  profileId: string;
  name: string;
  requests: number;
  spend: number | null;
  models: string[];
  durationMs: number;
}

function demoRuns(data: LensDemoData, window: TimeWindow, agent: string): DemoRun[] {
  return data.runs.flatMap((run) => {
    const trace = run.trace.summary;
    const startMs = Date.parse(trace.start_time);
    if (startMs < window.startMs || startMs >= window.endMs) return [];
    if (agent && !traceAgentNames(trace).includes(agent)) return [];
    const category = run.details.map((detail) => usageCategory(detail.attributes)).find(Boolean);
    return [
      {
        traceId: trace.trace_id,
        startMs,
        preview: trace.input_preview,
        profileId: category ? `recorded:${category}` : "unclassified",
        name: category ?? "Unclassified",
        requests: trace.llm_calls,
        spend: trace.spend,
        models: trace.models,
        durationMs: trace.duration_ms,
      },
    ];
  });
}

function summaries(runs: DemoRun[]): ProfileSummary[] {
  const groups = new Map<string, DemoRun[]>();
  for (const run of runs) groups.set(run.profileId, [...(groups.get(run.profileId) ?? []), run]);
  const ranked = [...groups.values()].sort((a, b) => b.length - a.length || a[0].profileId.localeCompare(b[0].profileId));
  return ranked.map((group, rank): ProfileSummary => {
    const priced = group.filter((run) => run.spend !== null);
    const angle = rank * 2.4;
    const radius = 34 * Math.sqrt((rank + 0.5) / ranked.length);
    return {
      profile_id: group[0].profileId,
      name: group[0].name,
      source: group[0].profileId === "unclassified" ? "unclassified" : "recorded",
      run_count: group.length,
      share: group.length / runs.length,
      request_count: group.reduce((sum, run) => sum + run.requests, 0),
      cost_observations: priced.length,
      spend_usd: priced.length ? priced.reduce((sum, run) => sum + (run.spend ?? 0), 0) : null,
      center: { x: 50 + radius * Math.cos(angle), y: 50 + radius * Math.sin(angle) },
    };
  });
}

function overview(runs: DemoRun[], window: TimeWindow, agent: string): ProfilesOverview {
  const profiles = summaries(runs);
  const centers = new Map(profiles.map((profile) => [profile.profile_id, profile.center]));
  const priced = runs.filter((run) => run.spend !== null);
  return {
    window: { agent, start_ms: window.startMs, end_ms: window.endMs },
    coverage: { runs_listed: runs.length, runs_resolved: runs.length, truncated: false },
    totals: {
      runs: runs.length,
      requests: runs.reduce((sum, run) => sum + run.requests, 0),
      matched_requests: priced.length,
      cost_observations: priced.length,
      spend_usd: priced.length ? priced.reduce((sum, run) => sum + (run.spend ?? 0), 0) : null,
      other_agent_requests: 0,
      shared_requests: 0,
    },
    profiles,
    points: runs.map((run, index) => {
      const center = centers.get(run.profileId) ?? { x: 50, y: 50 };
      const angle = index * 1.7;
      return { trace_id: run.traceId, profile_id: run.profileId, x: center.x + 4 * Math.cos(angle), y: center.y + 4 * Math.sin(angle), placement: "categorical" as const };
    }),
    taxonomy: null,
  };
}

function cells(runs: DemoRun[]): MatrixCell[] {
  const grouped = new Map<string, DemoRun[]>();
  for (const run of runs)
    for (const model of run.models) {
      const key = `${run.profileId}\u0000group:${model}`;
      grouped.set(key, [...(grouped.get(key) ?? []), run]);
    }
  return [...grouped].map(([key, group]) => {
    const [profileId, columnKey] = key.split("\u0000");
    const priced = group.filter((run) => run.spend !== null);
    return {
      profile_id: profileId,
      column_key: columnKey,
      request_count: group.length,
      avg_cost_usd: priced.length ? priced.reduce((sum, run) => sum + (run.spend ?? 0), 0) / priced.length : null,
      cost_observations: priced.length,
      p95_latency_ms: null,
      latency_observations: 0,
      eval_success_rate: null,
      eval_sample_count: 0,
    };
  });
}

export function demoProfilesApi(data: LensDemoData, usage: ProfilesApi["usage"]): ProfilesApi {
  return {
    scope: "demo",
    usage,
    overview: async (window, agent) => overview(demoRuns(data, window, agent), window, agent),
    matrix: async (window, agent) => {
      const runs = demoRuns(data, window, agent);
      const models = [...new Set(runs.flatMap((run) => run.models))].sort();
      return {
        window: { agent, start_ms: window.startMs, end_ms: window.endMs },
        coverage: { runs_listed: runs.length, runs_resolved: runs.length, truncated: false },
        profiles: summaries(runs).map((profile) => ({
          profile_id: profile.profile_id,
          name: profile.name,
          source: profile.source,
          required_capabilities: null,
        })),
        models: models.map((model) => ({
          column_key: `group:${model}`,
          model_config_id: null,
          model_name: model,
          model_group: model,
          provider: "",
          configured: false,
          capabilities: { function_calling: null, vision: null, reasoning: null, structured_output: null },
        })),
        cells: cells(runs),
        compatibility: [],
        other_agent_requests: 0,
      };
    },
    detail: async (window, agent, profileId) => {
      const runs = demoRuns(data, window, agent);
      const profile = summaries(runs).find((item) => item.profile_id === profileId || item.name === profileId);
      if (!profile) throw new Error("Profile not found in this agent and time range");
      const own = runs.filter((run) => run.profileId === profile.profile_id);
      return {
        window: { agent, start_ms: window.startMs, end_ms: window.endMs },
        coverage: { runs_listed: runs.length, runs_resolved: runs.length, truncated: false },
        profile,
        definition: null,
        runs: own.map((run) => ({
          trace_id: run.traceId,
          trace_ref: "",
          start_ms: run.startMs,
          input_preview: run.preview,
          request_count: run.requests,
          spend_usd: run.spend,
        })),
        models: cells(own),
      };
    },
    taxonomy: async () => ({ taxonomy: null, active: null, discovery_available: false, can_write: false }),
    settings: async (agent) => ({
      settings: { agent, enabled: false, models: { discovery: "", embedding: "", classification: "" }, revision: 0, updated_at: null },
      defaults: { discovery: "", embedding: "", classification: "" },
      options: { discovery: [], embedding: [], classification: [] },
      can_write: false,
    }),
    saveSettings: readOnly,
    preview: readOnly,
    discover: readOnly,
    editTask: readOnly,
    activate: readOnly,
    resume: readOnly,
    reset: readOnly,
  };
}
