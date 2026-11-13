import { apiClient } from "../../../lib/http/requests";
import type { TimeWindow } from "../../shared/timeRange/timeRange";
import type { TraceQueryBody } from "../traces/types";
import {
  detailSchema,
  matrixSchema,
  overviewSchema,
  type ProfileDetail,
  type ProfilesMatrix,
  type ProfilesOverview,
} from "./schemas";
import { usageProfileSchema, usageProfileSql, type UsageCategory } from "./usageProfile";

export interface ProfilesApi {
  readonly scope: string;
  usage(window: TimeWindow, agent: string, signal?: AbortSignal): Promise<UsageCategory[]>;
  overview(window: TimeWindow, agent: string, signal?: AbortSignal): Promise<ProfilesOverview>;
  matrix(window: TimeWindow, agent: string, signal?: AbortSignal): Promise<ProfilesMatrix>;
  detail(window: TimeWindow, agent: string, profileId: string, signal?: AbortSignal): Promise<ProfileDetail>;
}

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
  };
}
