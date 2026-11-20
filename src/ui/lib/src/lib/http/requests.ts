import { createApiClient } from "./client";
import { getAuthHeaderName, getRequestBaseUrl, reportError } from "./runtime";
import type {
  Trace,
  TraceFilters,
  TracePage,
  SpanDetail,
  SpanErrorPage,
  TraceListQuery,
  TraceDetailQuery,
  SpanQuery,
  SpanErrorQuery,
  LogListRequest,
  OtelLogPage,
  OtelLogRecord,
} from "../../components/lens/traces/types";

export const getProxyBaseUrl = getRequestBaseUrl;
export const apiClient = createApiClient({
  getBaseUrl: getRequestBaseUrl,
  getAuthHeaderName,
  onError: reportError,
});
export const modelCostMap = (
  catalogOnly = false,
): Promise<Record<string, unknown>> =>
  apiClient.get("/public/litellm_model_cost_map", {
    query: { catalog_only: catalogOnly || undefined },
  });

export const agentTraceListCall = async ({
  accessToken,
  startMs,
  endMs,
  cursor,
  agent,
  signal,
  filters = {},
}: {
  accessToken: string;
  startMs: number;
  endMs: number;
  cursor?: string | null;
  agent?: string;
  signal?: AbortSignal;
  filters?: TraceFilters;
}): Promise<TracePage> => {
  const query = {
    ...filters,
    start_ms: startMs,
    end_ms: endMs,
    cursor: cursor ?? undefined,
    agent: agent || undefined,
  } satisfies TraceListQuery;
  return apiClient.get<TracePage>(`/v1/traces`, { accessToken, query, signal });
};

export const agentTraceCall = async (
  accessToken: string,
  traceId: string,
  traceRef?: string,
  cursor?: string | null,
  filters: TraceFilters = {},
): Promise<Trace> =>
  apiClient.get<Trace>(`/v1/traces/${encodeURIComponent(traceId)}`, {
    accessToken,
    query: {
      ...filters,
      trace_ref: traceRef || undefined,
      cursor: cursor ?? undefined,
      page_size: 200,
    } satisfies TraceDetailQuery,
  });

export const agentTraceSpanCall = async (
  accessToken: string,
  traceId: string,
  spanId: string,
  traceRef?: string,
  signal?: AbortSignal,
  filters: TraceFilters = {},
): Promise<SpanDetail> =>
  apiClient.get<SpanDetail>(
    `/v1/traces/${encodeURIComponent(traceId)}/spans/${encodeURIComponent(spanId)}`,
    {
      accessToken,
      query: {
        ...filters,
        trace_ref: traceRef || undefined,
      } satisfies SpanQuery,
      signal,
    },
  );

export const agentTraceSpanErrorCall = async (
  accessToken: string,
  traceId: string,
  spanId: string,
  options: { traceRef?: string; cursor?: string | null },
  filters: TraceFilters = {},
): Promise<SpanErrorPage> =>
  apiClient.get<SpanErrorPage>(
    `/v1/traces/${encodeURIComponent(traceId)}/spans/${encodeURIComponent(spanId)}/error`,
    {
      accessToken,
      query: {
        ...filters,
        trace_ref: options.traceRef || undefined,
        cursor: options.cursor || undefined,
      } satisfies SpanErrorQuery,
    },
  );

export const logListCall = async ({
  accessToken,
  startMs,
  endMs,
  traceId,
  cursor,
  signal,
}: {
  accessToken: string;
  startMs: number;
  endMs: number;
  traceId?: string;
  cursor?: string | null;
  signal?: AbortSignal;
}): Promise<OtelLogPage> => {
  const query = {
    start_ms: startMs,
    end_ms: endMs,
    trace_id: traceId || undefined,
    cursor: cursor ?? undefined,
    page_size: 50,
  } satisfies LogListRequest;
  return apiClient.get<OtelLogPage>("/lens/logs", {
    accessToken,
    query,
    signal,
  });
};

export const logDetailCall = (
  accessToken: string,
  recordId: string,
  signal?: AbortSignal,
): Promise<OtelLogRecord> =>
  apiClient.get<OtelLogRecord>(`/lens/logs/${encodeURIComponent(recordId)}`, {
    accessToken,
    signal,
  });
