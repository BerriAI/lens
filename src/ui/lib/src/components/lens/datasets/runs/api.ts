"use client";

import { queryOptions, useQuery } from "@tanstack/react-query";
import { useLensApi } from "../../data/LensServices";
import { datasetKeys } from "../api";
import type { EvalRunsApi } from "./client";
import type { EvalRunFilter } from "./types";

const evalRunKeys = {
  all: () => [...datasetKeys.all(), "evalRuns"] as const,
  list: (scope: string, filter: EvalRunFilter) => [...evalRunKeys.all(), "list", { scope, ...filter }] as const,
  detail: (scope: string, runId: string) => [...evalRunKeys.all(), "detail", { scope, runId }] as const,
  runCase: (scope: string, runId: string, caseId: string) =>
    [...evalRunKeys.all(), "case", { scope, runId, caseId }] as const,
};

const evalRunQueries = {
  list(api: EvalRunsApi, scope: string, filter: EvalRunFilter) {
    return queryOptions({ queryKey: evalRunKeys.list(scope, filter), queryFn: () => api.list(filter) });
  },
  detail(api: EvalRunsApi, scope: string, runId: string) {
    return queryOptions({ queryKey: evalRunKeys.detail(scope, runId), queryFn: () => api.get(runId) });
  },
  runCase(api: EvalRunsApi, scope: string, runId: string | null, caseId: string) {
    return queryOptions({
      queryKey: evalRunKeys.runCase(scope, runId ?? "", caseId),
      queryFn: () => api.runCase(runId ?? "", caseId),
      enabled: runId !== null,
      staleTime: Infinity,
    });
  },
};

export function useEvalRuns(filter: EvalRunFilter) {
  const api = useLensApi();
  return useQuery(evalRunQueries.list(api.evalRuns, api.scope, filter));
}

export function useEvalRun(runId: string) {
  const api = useLensApi();
  return useQuery(evalRunQueries.detail(api.evalRuns, api.scope, runId));
}

export function useRunCase(runId: string | null, caseId: string) {
  const api = useLensApi();
  return useQuery(evalRunQueries.runCase(api.evalRuns, api.scope, runId, caseId));
}
