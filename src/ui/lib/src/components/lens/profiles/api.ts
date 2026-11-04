import { apiClient } from "../../../lib/http/requests";
import type { TimeWindow } from "../../shared/timeRange/timeRange";
import type { TraceQueryBody } from "../traces/types";
import { usageProfileSchema, usageProfileSql, type UsageCategory } from "./usageProfile";

export interface ProfilesApi {
  readonly scope: string;
  usage(window: TimeWindow, agent: string, signal?: AbortSignal): Promise<UsageCategory[]>;
}

export function liveProfilesApi(accessToken: string): ProfilesApi {
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
  };
}
