import { createApiClient } from "./client";
import { getAuthHeaderName, getRequestBaseUrl, reportError } from "./runtime";
import type {
  Trace,
  TracePage,
  SpanDetail,
  SpanErrorPage,
  TraceListQuery,
  TraceDetailQuery,
  SpanQuery,
  SpanErrorQuery,
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
}: {
  accessToken: string;
  startMs: number;
  endMs: number;
  cursor?: string | null;
}): Promise<TracePage> => {
  const query = {
    start_ms: startMs,
    end_ms: endMs,
    cursor: cursor ?? undefined,
  } satisfies TraceListQuery;
  return apiClient.get<TracePage>(`/v1/traces`, { accessToken, query });
};

export const agentTraceCall = async (
  accessToken: string,
  traceId: string,
  traceRef?: string,
  cursor?: string | null,
): Promise<Trace> =>
  apiClient.get<Trace>(`/v1/traces/${encodeURIComponent(traceId)}`, {
    accessToken,
    query: {
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
): Promise<SpanDetail> =>
  apiClient.get<SpanDetail>(
    `/v1/traces/${encodeURIComponent(traceId)}/spans/${encodeURIComponent(spanId)}`,
    {
      accessToken,
      query: { trace_ref: traceRef || undefined } satisfies SpanQuery,
    },
  );

export const agentTraceSpanErrorCall = async (
  accessToken: string,
  traceId: string,
  spanId: string,
  options: { traceRef?: string; cursor?: string | null },
): Promise<SpanErrorPage> =>
  apiClient.get<SpanErrorPage>(
    `/v1/traces/${encodeURIComponent(traceId)}/spans/${encodeURIComponent(spanId)}/error`,
    {
      accessToken,
      query: {
        trace_ref: options.traceRef || undefined,
        cursor: options.cursor || undefined,
      } satisfies SpanErrorQuery,
    },
  );
