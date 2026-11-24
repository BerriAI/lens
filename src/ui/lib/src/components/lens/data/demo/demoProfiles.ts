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
  calls: DemoCall[];
  spend: number | null;
}

interface DemoCall {
  model: string;
  spend: number | null;
}

const pricedSpend = (calls: readonly DemoCall[]): number | null => {
  const priced = calls.flatMap((call) => (call.spend === null ? [] : [call.spend]));
  return priced.length ? priced.reduce((sum, spend) => sum + spend, 0) : null;
};

const priced = (runs: readonly DemoRun[]) => runs.flatMap((run) => run.calls).filter((call) => call.spend !== null).length;
const requests = (runs: readonly DemoRun[]) => runs.reduce((sum, run) => sum + run.calls.length, 0);
const spendOf = (runs: readonly DemoRun[]) => pricedSpend(runs.flatMap((run) => run.calls));

function demoRuns(data: LensDemoData, window: TimeWindow, agent: string): DemoRun[] {
  return data.runs.flatMap((run) => {
    const trace = run.trace.summary;
    const startMs = Date.parse(trace.start_time);
    if (startMs < window.startMs || startMs >= window.endMs) return [];
    if (agent && !traceAgentNames(trace).includes(agent)) return [];
    const category = run.details.map((detail) => usageCategory(detail.attributes)).find(Boolean);
    const calls = run.trace.spans
      .filter((span) => span.type === "llm")
      .map((span) => ({ model: span.model ?? "", spend: span.spend }));
    return [
      {
        traceId: trace.trace_id,
        startMs,
        preview: trace.input_preview,
        profileId: category ? `recorded:${category}` : "unclassified",
        name: category ?? "Unclassified",
        calls,
        spend: pricedSpend(calls),
      },
    ];
  });
}

function summaries(runs: DemoRun[]): ProfileSummary[] {
  const groups = new Map<string, DemoRun[]>();
  for (const run of runs) groups.set(run.profileId, [...(groups.get(run.profileId) ?? []), run]);
  const ranked = [...groups.values()].sort((a, b) => b.length - a.length || a[0].profileId.localeCompare(b[0].profileId));
  return ranked.map(
    (group): ProfileSummary => ({
      profile_id: group[0].profileId,
      name: group[0].name,
      source: group[0].profileId === "unclassified" ? "unclassified" : "recorded",
      run_count: group.length,
      share: group.length / runs.length,
      request_count: requests(group),
      cost_observations: priced(group),
      spend_usd: spendOf(group),
    }),
  );
}

function overview(runs: DemoRun[], window: TimeWindow, agent: string): ProfilesOverview {
  return {
    window: { agent, start_ms: window.startMs, end_ms: window.endMs },
    coverage: { runs_listed: runs.length, runs_resolved: runs.length, truncated: false },
    totals: {
      runs: runs.length,
      requests: requests(runs),
      matched_requests: priced(runs),
      cost_observations: priced(runs),
      spend_usd: spendOf(runs),
      other_agent_requests: 0,
      shared_requests: 0,
    },
    profiles: summaries(runs),
    points: [],
  };
}

function cells(runs: DemoRun[]): MatrixCell[] {
  const grouped = new Map<string, DemoCall[]>();
  for (const run of runs)
    for (const call of run.calls) {
      const key = `${run.profileId}\u0000${call.model ? `group:${call.model}` : "unmatched:"}`;
      grouped.set(key, [...(grouped.get(key) ?? []), call]);
    }
  return [...grouped].map(([key, calls]) => {
    const [profileId, columnKey] = key.split("\u0000");
    const observations = calls.filter((call) => call.spend !== null).length;
    const spend = pricedSpend(calls);
    return {
      profile_id: profileId,
      column_key: columnKey,
      request_count: calls.length,
      avg_cost_usd: spend === null ? null : spend / observations,
      cost_observations: observations,
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
      const matrixCells = cells(runs);
      const columns = [...new Set(matrixCells.map((cell) => cell.column_key))].sort();
      return {
        window: { agent, start_ms: window.startMs, end_ms: window.endMs },
        coverage: { runs_listed: runs.length, runs_resolved: runs.length, truncated: false },
        profiles: summaries(runs).map((profile) => ({
          profile_id: profile.profile_id,
          name: profile.name,
          source: profile.source,
          required_capabilities: null,
        })),
        models: columns.map((column) => {
          const model = column.replace(/^(group|unmatched):/, "");
          return {
            column_key: column,
            model_config_id: null,
            model_name: model,
            model_group: model,
            provider: "",
            configured: false,
            capabilities: { function_calling: null, vision: null, reasoning: null, structured_output: null },
          };
        }),
        cells: matrixCells,
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
          request_count: run.calls.length,
          spend_usd: run.spend,
          resolved: true,
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
    activate: readOnly,
    resume: readOnly,
    reset: readOnly,
  };
}
