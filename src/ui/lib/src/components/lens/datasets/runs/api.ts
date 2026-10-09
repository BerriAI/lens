"use client";

import { queryOptions, useQuery } from "@tanstack/react-query";
import { useLensAccessToken, useLensApi } from "../../data/LensServices";
import { useTracesApi, type TracesApi } from "../../traces/api";
import type { Span, Trace } from "../../traces/types";
import { datasetKeys } from "../api";
import type { EvalRunsApi } from "./client";
import type { CaseOutcome, EvalRunFilter } from "./types";

const evalRunKeys = {
  all: () => [...datasetKeys.all(), "evalRuns"] as const,
  list: (scope: string, filter: EvalRunFilter) => [...evalRunKeys.all(), "list", { scope, ...filter }] as const,
  detail: (scope: string, runId: string) => [...evalRunKeys.all(), "detail", { scope, runId }] as const,
  trace: (scope: string, traceId: string, traceRef: string) =>
    [...evalRunKeys.all(), "trace", { scope, traceId, traceRef }] as const,
};

export const MAX_TRACE_PAGES = 50;

export async function fullTrace(
  api: TracesApi,
  outcome: CaseOutcome,
  cursors: readonly string[] = [],
  earlier: readonly Span[] = [],
): Promise<Trace> {
  const page = await api.trace(outcome.trace_id, outcome.trace_ref || undefined, cursors.at(-1) ?? null);
  const spans = [...earlier, ...page.spans];
  const next = page.next_cursor;
  if (!next) return { ...page, spans };
  if (cursors.includes(next) || cursors.length + 1 >= MAX_TRACE_PAGES)
    throw new Error(`Trace ${outcome.trace_id} kept paging past ${cursors.length + 1} pages`);
  return fullTrace(api, outcome, [...cursors, next], spans);
}

const evalRunQueries = {
  list(api: EvalRunsApi, scope: string, filter: EvalRunFilter) {
    return queryOptions({ queryKey: evalRunKeys.list(scope, filter), queryFn: () => api.list(filter) });
  },
  detail(api: EvalRunsApi, scope: string, runId: string) {
    return queryOptions({ queryKey: evalRunKeys.detail(scope, runId), queryFn: () => api.get(runId) });
  },
  trace(api: TracesApi, scope: string, outcome: CaseOutcome | null) {
    const options = {
      queryKey: evalRunKeys.trace(scope, outcome?.trace_id ?? "", outcome?.trace_ref ?? ""),
      queryFn: () => (outcome ? fullTrace(api, outcome) : null),
      enabled: !!outcome?.trace_id,
      staleTime: Infinity,
    };
    return queryOptions(options);
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

export function useCaseTrace(outcome: CaseOutcome | null) {
  const traces = useTracesApi(useLensAccessToken());
  return useQuery(evalRunQueries.trace(traces, useLensApi().scope, outcome));
}
