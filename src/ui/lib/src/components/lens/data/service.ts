import type { ApiClient } from "../../../lib/http/client";
import { authHeaders } from "../../../lib/http/authHeaders";
import type { Client } from "openapi-fetch";
import type { components, paths } from "../../../lib/http/schema";
import { liveDatasetsApi, type DatasetsApi } from "../datasets/client";
import { liveEvalRunsApi, type EvalRunsApi } from "../evals/runs/client";
import type {
  ActivitySelection,
  AnalysisModelInfo,
  GatewayStatus,
  Job,
  Lens,
  LensList,
  RunWindow,
  Sample,
  Settings,
  SignalConfig,
} from "../model/types";

export type ExecutionContent = components["schemas"]["ExecutionContent"];
export type FindingStatus = components["schemas"]["FindingUpdate"]["status"];

export type ReviewPage = components["schemas"]["ReviewPage"];

export interface LensApi {
  /** Partitions query caches between backends (one token, or the demo). */
  readonly scope: string;
  readonly datasets: DatasetsApi;
  readonly evalRuns: EvalRunsApi;
  lenses(): Promise<LensList>;
  activity(): Promise<components["schemas"]["ActivityAvailability"]>;
  runs(lensId: string, offset: number): Promise<Job[]>;
  run(lensId: string, jobId: string): Promise<Job>;
  reviews(lensId: string, jobId: string, after: number): Promise<ReviewPage>;
  execution(lensId: string, executionId: string, offset: number): Promise<ExecutionContent>;
  sample(selection: ActivitySelection, offset: number, asOf: string): Promise<Sample>;
  agents(): Promise<string[]>;
  models(): Promise<{ data: { id: string }[] }>;
  modelDetails(): Promise<{ data: AnalysisModelInfo[] }>;
  gateway(): Promise<GatewayStatus>;
  refreshGateway(): Promise<GatewayStatus>;
  saveLens(id: string | undefined, settings: Settings): Promise<Lens>;
  startRun(lensId: string, request?: RunWindow): Promise<void>;
  watchAll(): Promise<components["schemas"]["WatchAllResult"]>;
  signalConfig(): Promise<SignalConfig>;
  saveSignalConfig(config: SignalConfig): Promise<SignalConfig>;
  cancelRun(lensId: string): Promise<void>;
  reviewFinding(lensId: string, findingId: string, status: FindingStatus, reason: string): Promise<void>;
}

type LensClient = Client<paths>;

async function required<T>(request: Promise<{ data?: T }>): Promise<T> {
  const { data } = await request;
  if (data === undefined) throw new Error("The proxy returned an empty response");
  return data;
}

async function sent(request: Promise<unknown>): Promise<void> {
  await request;
}

export function liveLensApi(client: LensClient, apiClient: ApiClient, accessToken: string): LensApi {
  const headers = authHeaders(accessToken);
  const lens = (lens_id: string) => ({
    headers,
    params: { path: { lens_id } },
  });
  return {
    scope: accessToken,
    datasets: liveDatasetsApi(client, apiClient, accessToken),
    evalRuns: liveEvalRunsApi(apiClient, accessToken),
    lenses: () => required(client.GET("/lens", { headers })),
    activity: () => required(client.GET("/lens/activity/available", { headers })),
    runs: (lensId, offset) =>
      required(
        client.GET("/lens/{lens_id}/runs", { headers, params: { path: { lens_id: lensId }, query: { offset } } }),
      ),
    run: (lensId, jobId) =>
      required(
        client.GET("/lens/{lens_id}/runs/{job_id}", {
          headers,
          params: { path: { lens_id: lensId, job_id: jobId } },
        }),
      ),
    reviews: (lensId, jobId, after) =>
      required(
        client.GET("/lens/{lens_id}/runs/{job_id}/reviews", {
          headers,
          params: { path: { lens_id: lensId, job_id: jobId }, query: { after } },
        }),
      ),
    execution: (lensId, executionId, offset) =>
      required(
        client.GET("/lens/{lens_id}/executions/{execution_id}", {
          headers,
          params: { path: { lens_id: lensId, execution_id: executionId }, query: { offset } },
        }),
      ),
    sample: (selection, offset, asOf) =>
      required(
        client.POST("/lens/preview/sample", {
          headers,
          body: {
            offset,
            as_of: asOf,
            selection: {
              source: selection.source,
              service: selection.service ?? "",
              agent_name: selection.agent_name ?? "",
              filters: selection.filters ?? [],
              sample_size: selection.sample_size,
              sample_percent: selection.sample_percent ?? 100,
              team_id: selection.team_id ?? "",
              execution_ids: [],
            },
            lookback_hours: selection.lookback_hours ?? 24,
          },
        }),
      ),
    agents: () => required(client.GET("/lens/agents", { headers })),
    models: () => apiClient.get("/lens/models", { accessToken }),
    modelDetails: () => apiClient.get("/lens/model_group/info", { accessToken }),
    gateway: () => apiClient.get("/lens/gateway", { accessToken }),
    refreshGateway: () => apiClient.post("/lens/gateway/refresh", { accessToken }),
    saveLens: (id, settings) =>
      required(
        id
          ? client.PUT("/lens/{lens_id}", { ...lens(id), body: settings })
          : client.POST("/lens", { headers, body: settings }),
      ),
    startRun: (lensId, request = {}) => sent(client.POST("/lens/{lens_id}/runs", { ...lens(lensId), body: request })),
    watchAll: () => required(client.POST("/lens/watch-all", { headers })),
    signalConfig: () => required(client.GET("/lens/signals", { headers })),
    saveSignalConfig: (config) => required(client.PUT("/lens/signals", { headers, body: config })),
    cancelRun: (lensId) => sent(client.POST("/lens/{lens_id}/cancel", lens(lensId))),
    reviewFinding: (lensId, findingId, status, reason) =>
      sent(
        client.PATCH("/lens/{lens_id}/findings/{finding_id}", {
          headers,
          params: { path: { lens_id: lensId, finding_id: findingId } },
          body: { status, reason },
        }),
      ),
  };
}
