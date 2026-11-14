import { apiClient } from "../../../lib/http/requests";
import type { TimeWindow } from "../../shared/timeRange/timeRange";
import type { TraceQueryBody } from "../traces/types";
import {
  detailSchema,
  matrixSchema,
  overviewSchema,
  previewSchema,
  settingsSchema,
  settingsViewSchema,
  taxonomyViewSchema,
  type DiscoveryPreview,
  type ModelChoice,
  type ProfileDetail,
  type ProfileSettings,
  type ProfileSettingsView,
  type ProfilesMatrix,
  type ProfilesOverview,
  type TaskEdit,
  type TaxonomyView,
} from "./schemas";
import { usageProfileSchema, usageProfileSql, type UsageCategory } from "./usageProfile";

export interface DiscoveryInput {
  agent: string;
  window: TimeWindow;
  maxRuns: number;
}

export interface SettingsUpdate {
  agent: string;
  revision: number;
  enabled: boolean;
  models: ModelChoice;
}

export type TaxonomyTarget = "draft" | "active";

export interface ProfilesApi {
  readonly scope: string;
  usage(window: TimeWindow, agent: string, signal?: AbortSignal): Promise<UsageCategory[]>;
  overview(window: TimeWindow, agent: string, signal?: AbortSignal): Promise<ProfilesOverview>;
  matrix(window: TimeWindow, agent: string, signal?: AbortSignal): Promise<ProfilesMatrix>;
  detail(window: TimeWindow, agent: string, profileId: string, signal?: AbortSignal): Promise<ProfileDetail>;
  taxonomy(agent: string, signal?: AbortSignal): Promise<TaxonomyView>;
  settings(agent: string, signal?: AbortSignal): Promise<ProfileSettingsView>;
  saveSettings(update: SettingsUpdate): Promise<ProfileSettings>;
  preview(input: DiscoveryInput, signal?: AbortSignal): Promise<DiscoveryPreview>;
  discover(input: DiscoveryInput, revision: number): Promise<TaxonomyView>;
  editTask(agent: string, revision: number, taskId: string, edit: TaskEdit): Promise<TaxonomyView>;
  activate(agent: string, revision: number, backfill: boolean): Promise<TaxonomyView>;
  resume(agent: string, revision: number): Promise<TaxonomyView>;
  reset(agent: string, revision: number, target: TaxonomyTarget): Promise<TaxonomyView>;
}

const discoveryBody = ({ agent, window, maxRuns }: DiscoveryInput) => ({
  agent,
  start_ms: Math.trunc(window.startMs),
  end_ms: Math.trunc(window.endMs),
  max_runs: maxRuns,
});

const windowQuery = (window: TimeWindow, agent: string) => ({
  agent,
  start_ms: String(Math.trunc(window.startMs)),
  end_ms: String(Math.trunc(window.endMs)),
});

export function liveProfilesApi(accessToken: string): ProfilesApi {
  const get = async <T>(path: string, query: Record<string, string>, parse: (value: unknown) => T, signal?: AbortSignal) =>
    parse(await apiClient.get<unknown>(path, { accessToken, query, signal }));
  return {
    scope: accessToken,
    usage: async (window, agent, signal) => {
      const response = await apiClient.post<unknown>("/v1/traces/query", {
        accessToken,
        signal,
        body: { sql: usageProfileSql(window, agent) } satisfies TraceQueryBody,
      });
      return usageProfileSchema.parse(response).data;
    },
    overview: (window, agent, signal) =>
      get("/lens/profiles/overview", windowQuery(window, agent), (value) => overviewSchema.parse(value), signal),
    matrix: (window, agent, signal) =>
      get("/lens/profiles/matrix", windowQuery(window, agent), (value) => matrixSchema.parse(value), signal),
    detail: (window, agent, profileId, signal) =>
      get(
        "/lens/profiles/detail",
        { ...windowQuery(window, agent), profile_id: profileId },
        (value) => detailSchema.parse(value),
        signal,
      ),
    taxonomy: (agent, signal) => get("/lens/profiles/taxonomy", { agent }, (value) => taxonomyViewSchema.parse(value), signal),
    settings: (agent, signal) => get("/lens/profiles/settings", { agent }, (value) => settingsViewSchema.parse(value), signal),
    saveSettings: async (update) =>
      settingsSchema.parse(await apiClient.put<unknown>("/lens/profiles/settings", { accessToken, body: update })),
    preview: async (input, signal) =>
      previewSchema.parse(
        await apiClient.post<unknown>("/lens/profiles/discovery/preview", { accessToken, signal, body: discoveryBody(input) }),
      ),
    discover: async (input, revision) =>
      taxonomyViewSchema.parse(
        await apiClient.post<unknown>("/lens/profiles/discovery", { accessToken, body: { ...discoveryBody(input), revision } }),
      ),
    editTask: async (agent, revision, taskId, edit) =>
      taxonomyViewSchema.parse(
        await apiClient.patch<unknown>(`/lens/profiles/taxonomy/tasks/${encodeURIComponent(taskId)}`, {
          accessToken,
          body: { agent, revision, ...edit },
        }),
      ),
    activate: async (agent, revision, backfill) =>
      taxonomyViewSchema.parse(
        await apiClient.post<unknown>("/lens/profiles/taxonomy/activate", { accessToken, body: { agent, revision, backfill } }),
      ),
    resume: async (agent, revision) =>
      taxonomyViewSchema.parse(
        await apiClient.post<unknown>("/lens/profiles/taxonomy/resume", { accessToken, body: { agent, revision } }),
      ),
    reset: async (agent, revision, target) =>
      taxonomyViewSchema.parse(
        await apiClient.post<unknown>("/lens/profiles/taxonomy/reset", { accessToken, body: { agent, revision, target } }),
      ),
  };
}
