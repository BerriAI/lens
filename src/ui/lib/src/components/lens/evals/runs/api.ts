"use client";

import {
  queryOptions,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { useLensApi } from "../../data/LensServices";
import { datasetKeys } from "../../datasets/api";
import type { EvalRunsApi } from "./client";
import type { EvalRunFilter, EvalSpec } from "./types";

const evalKeys = {
  all: () => [...datasetKeys.all(), "evals"] as const,
  list: (scope: string) => [...evalKeys.all(), "list", { scope }] as const,
  detail: (scope: string, name: string) => [...evalKeys.all(), "detail", { scope, name }] as const,
};

export function useEvals() {
  const api = useLensApi();
  return useQuery({
    queryKey: evalKeys.list(api.scope),
    queryFn: () => api.evalRuns.evals(),
  });
}

export function useEvalDefinition(name: string) {
  const api = useLensApi();
  return useQuery({
    queryKey: evalKeys.detail(api.scope, name),
    queryFn: () => api.evalRuns.evalDefinition(name),
  });
}

export function useSaveEval() {
  const api = useLensApi();
  const client = useQueryClient();
  return useMutation({
    retry: false,
    mutationFn: ({ name, spec }: { name: string; spec: EvalSpec }) => api.evalRuns.saveEval(name, spec),
    onSettled: () => client.invalidateQueries({ queryKey: evalKeys.all() }),
  });
}

const evalRunKeys = {
  all: () => [...datasetKeys.all(), "evalRuns"] as const,
  list: (scope: string, filter: EvalRunFilter) =>
    [...evalRunKeys.all(), "list", { scope, ...filter }] as const,
  detail: (scope: string, runId: string) =>
    [...evalRunKeys.all(), "detail", { scope, runId }] as const,
  cases: (scope: string, runId: string) =>
    [...evalRunKeys.all(), "cases", { scope, runId }] as const,
  runCase: (scope: string, runId: string, caseId: string) =>
    [...evalRunKeys.all(), "case", { scope, runId, caseId }] as const,
};

const evalRunQueries = {
  list(api: EvalRunsApi, scope: string, filter: EvalRunFilter) {
    return queryOptions({
      queryKey: evalRunKeys.list(scope, filter),
      queryFn: () => api.list(filter),
    });
  },
  detail(api: EvalRunsApi, scope: string, runId: string) {
    return queryOptions({
      queryKey: evalRunKeys.detail(scope, runId),
      queryFn: () => api.get(runId),
    });
  },
  runCase(
    api: EvalRunsApi,
    scope: string,
    runId: string | null,
    caseId: string,
  ) {
    return queryOptions({
      queryKey: evalRunKeys.runCase(scope, runId ?? "", caseId),
      queryFn: () => api.runCase(runId ?? "", caseId),
      enabled: runId !== null,
      staleTime: Infinity,
    });
  },
};

export const EMPTY_RUNS_POLL_MS = 5000;

export function useEvalRuns(filter: EvalRunFilter) {
  const api = useLensApi();
  return useQuery({
    ...evalRunQueries.list(api.evalRuns, api.scope, filter),
    refetchInterval: (query) =>
      query.state.data?.length === 0 ? EMPTY_RUNS_POLL_MS : false,
  });
}

export function useEvalRun(runId: string) {
  const api = useLensApi();
  return useQuery(evalRunQueries.detail(api.evalRuns, api.scope, runId));
}

export function useRunCase(runId: string | null, caseId: string) {
  const api = useLensApi();
  return useQuery(
    evalRunQueries.runCase(api.evalRuns, api.scope, runId, caseId),
  );
}

export function useRunCases(runId: string) {
  const api = useLensApi();
  return useQuery({
    queryKey: evalRunKeys.cases(api.scope, runId),
    queryFn: () => api.evalRuns.cases(runId),
  });
}
