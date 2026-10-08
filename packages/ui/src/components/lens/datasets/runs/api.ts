"use client";

import { queryOptions, useQuery } from "@tanstack/react-query";
import { useMemo } from "react";

import { apiClient } from "@/components/networking";

import { useLensAccessToken, useLensApi } from "../../data/LensServices";
import { useTracesApi, type TracesApi } from "../../traces/api";
import type { Span, Trace } from "../../traces/types";
import { datasetKeys } from "../api";
import { liveEvalRunsApi, type EvalRunsApi } from "./client";
import type { CaseOutcome, EvalRunFilter } from "./types";

const evalRunKeys = {
  all: () => [...datasetKeys.all(), "evalRuns"] as const,
  list: (scope: string, filter: EvalRunFilter) => [...evalRunKeys.all(), "list", { scope, ...filter }] as const,
  detail: (scope: string, runId: string) => [...evalRunKeys.all(), "detail", { scope, runId }] as const,
  trace: (scope: string, traceId: string, traceRef: string) =>
    [...evalRunKeys.all(), "trace", { scope, traceId, traceRef }] as const,
};

async function fullTrace(
  api: TracesApi,
  outcome: CaseOutcome,
  cursor: string | null = null,
  earlier: readonly Span[] = [],
): Promise<Trace> {
  const page = await api.trace(outcome.trace_id, outcome.trace_ref || undefined, cursor);
  const spans = [...earlier, ...page.spans];
  return page.next_cursor ? fullTrace(api, outcome, page.next_cursor, spans) : { ...page, spans };
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
      enabled: outcome !== null,
      staleTime: Infinity,
    };
    return queryOptions(options);
  },
};

function useEvalRunsApi(): EvalRunsApi {
  const accessToken = useLensAccessToken();
  return useMemo(() => liveEvalRunsApi(apiClient, accessToken), [accessToken]);
}

export function useEvalRuns(filter: EvalRunFilter) {
  const api = useEvalRunsApi();
  return useQuery(evalRunQueries.list(api, useLensApi().scope, filter));
}

export function useEvalRun(runId: string) {
  const api = useEvalRunsApi();
  return useQuery(evalRunQueries.detail(api, useLensApi().scope, runId));
}

export function useCaseTrace(outcome: CaseOutcome | null) {
  const traces = useTracesApi(useLensAccessToken());
  return useQuery(evalRunQueries.trace(traces, useLensApi().scope, outcome));
}
