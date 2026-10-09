import type { ApiClient } from "../../../../lib/http/client";

import type { EvalRun, EvalRunFilter } from "./types";
import { evalRun, type RunDetails } from "./contract";

export interface EvalRunsApi {
  list(filter: EvalRunFilter): Promise<readonly EvalRun[]>;
  get(runId: string): Promise<EvalRun>;
}

const evalRunPath = (runId: string): string =>
  `/lens/evals/runs/${encodeURIComponent(runId)}/details`;

export function liveEvalRunsApi(
  apiClient: ApiClient,
  accessToken: string,
): EvalRunsApi {
  return {
    list: async (filter) =>
      (
        await apiClient.get<RunDetails[]>("/lens/evals/runs/details", {
          accessToken,
          query: { ...filter },
          headers: { "X-Lens-Contract": "1" },
        })
      ).map(evalRun),
    get: async (runId) =>
      evalRun(
        await apiClient.get<RunDetails>(evalRunPath(runId), {
          accessToken,
          headers: { "X-Lens-Contract": "1" },
        }),
      ),
  };
}
