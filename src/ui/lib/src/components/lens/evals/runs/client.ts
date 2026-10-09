import type { ApiClient } from "../../../../lib/http/client";

import type {
  EvalDefinition,
  EvalRun,
  EvalRunFilter,
  EvalSpec,
  RunCase,
  RunCaseSummary,
} from "./types";

export interface EvalRunsApi {
  list(filter: EvalRunFilter): Promise<readonly EvalRun[]>;
  get(runId: string): Promise<EvalRun>;
  cases(runId: string): Promise<readonly RunCaseSummary[]>;
  runCase(runId: string, caseId: string): Promise<RunCase>;
  evals(): Promise<readonly EvalDefinition[]>;
  evalDefinition(name: string): Promise<EvalDefinition>;
  saveEval(name: string, spec: EvalSpec): Promise<EvalDefinition>;
}

const CONTRACT = { "X-Lens-Contract": "1" };

const evalRunPath = (runId: string): string =>
  `/lens/evals/runs/${encodeURIComponent(runId)}`;

export function liveEvalRunsApi(
  apiClient: ApiClient,
  accessToken: string,
): EvalRunsApi {
  const evalPath = (name: string) => `/lens/evals/${encodeURIComponent(name)}`;
  return {
    evals: () =>
      apiClient.get<EvalDefinition[]>("/lens/evals", {
        accessToken,
        headers: CONTRACT,
      }),
    evalDefinition: (name) =>
      apiClient.get<EvalDefinition>(evalPath(name), {
        accessToken,
        headers: CONTRACT,
      }),
    saveEval: (name, spec) =>
      apiClient.put<EvalDefinition>(evalPath(name), {
        accessToken,
        headers: CONTRACT,
        body: spec,
      }),
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
    cases: (runId) =>
      apiClient.get<RunCaseSummary[]>(`${evalRunPath(runId)}/cases`, {
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
