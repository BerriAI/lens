import type { ApiClient } from "@/lib/http/client";

import type { EvalRun, EvalRunFilter } from "./types";

export interface EvalRunsApi {
  list(filter: EvalRunFilter): Promise<readonly EvalRun[]>;
  get(runId: string): Promise<EvalRun>;
}

const evalRunPath = (runId: string): string => `/lens/evals/runs/${encodeURIComponent(runId)}`;

export function liveEvalRunsApi(apiClient: ApiClient, accessToken: string): EvalRunsApi {
  return {
    list: (filter) => apiClient.get<EvalRun[]>("/lens/evals/runs", { accessToken, query: { ...filter } }),
    get: (runId) => apiClient.get<EvalRun>(evalRunPath(runId), { accessToken }),
  };
}
