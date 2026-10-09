import type { ApiClient } from "../../../../lib/http/client";

import type { EvalRun, EvalRunFilter, RunCase } from "./types";

export interface EvalRunsApi {
  list(filter: EvalRunFilter): Promise<readonly EvalRun[]>;
  get(runId: string): Promise<EvalRun>;
  runCase(runId: string, caseId: string): Promise<RunCase>;
}

const CONTRACT = { "X-Lens-Contract": "1" };

const evalRunPath = (runId: string): string =>
  `/lens/evals/runs/${encodeURIComponent(runId)}`;

export function liveEvalRunsApi(
  apiClient: ApiClient,
  accessToken: string,
): EvalRunsApi {
  return {
    list: (filter) =>
      apiClient.get<EvalRun[]>("/lens/evals/runs", {
        accessToken,
        headers: CONTRACT,
        query: { ...filter },
      }),
    get: (runId) =>
      apiClient.get<EvalRun>(evalRunPath(runId), {
        accessToken,
        headers: CONTRACT,
      }),
    runCase: (runId, caseId) =>
      apiClient.get<RunCase>(
        `${evalRunPath(runId)}/cases/${encodeURIComponent(caseId)}`,
        {
          accessToken,
          headers: CONTRACT,
        },
      ),
  };
}
