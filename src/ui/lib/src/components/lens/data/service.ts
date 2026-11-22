import type { ApiClient } from "../../../lib/http/client";
import { authHeaders } from "../../../lib/http/authHeaders";
import type { Client } from "openapi-fetch";
import type { components, paths } from "../../../lib/http/schema";
import { liveDatasetsApi, type DatasetsApi } from "../datasets/client";
import { liveEvalRunsApi, type EvalRunsApi } from "../evals/runs/client";
import type {
  ActivitySelection,
  AlertRuleConfig,
  AlertRuleOverview,
  AnalysisModelInfo,
  GatewayStatus,
  Job,
  Lens,
  LensList,
  RunWindow,
  Sample,
  Settings,
  SignalConfig,
  SignalOverview,
  SignalAlertView,
  SignalAlertRequest,
  SignalAlertTest,
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
  signalOverview(): Promise<SignalOverview>;
  alertRules(): Promise<AlertRuleConfig>;
  saveAlertRules(config: AlertRuleConfig): Promise<AlertRuleConfig>;
  alertRuleOverview(): Promise<AlertRuleOverview>;
  saveSignalConfig(config: SignalConfig): Promise<SignalConfig>;
  signalAlerts(): Promise<SignalAlertView>;
  saveSignalAlerts(config: SignalAlertRequest): Promise<SignalAlertView>;
  testSignalAlerts(config: SignalAlertTest): Promise<{ delivered: boolean }>;
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
    activity: () => required(client.GET("/lens/admin/activity/available", { headers })),
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
        client.GET("/lens/admin/{lens_id}/executions/{execution_id}", {
          headers,
          params: { path: { lens_id: lensId, execution_id: executionId }, query: { offset } },
        }),
      ),
    sample: (selection, offset, asOf) =>
      required(
        client.POST("/lens/admin/preview/sample", {
          headers,
          body: {
            offset,
            as_of: asOf,
            selection: {
              source: selection.source,
              service: selection.service ?? "",
              agent_name: selection.agent_name ?? "",
              ...(selection.profile ? { profile: selection.profile } : {}),
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
    agents: () => required(client.GET("/lens/admin/agents", { headers })),
    models: () => apiClient.get("/lens/admin/models", { accessToken }),
    modelDetails: () => apiClient.get("/lens/admin/model_group/info", { accessToken }),
    gateway: () => apiClient.get("/lens/admin/gateway", { accessToken }),
    refreshGateway: () => apiClient.post("/lens/admin/gateway/refresh", { accessToken }),
    saveLens: (id, settings) =>
      required(
        id
          ? client.PUT("/lens/{lens_id}", { ...lens(id), body: settings })
          : client.POST("/lens", { headers, body: settings }),
      ),
    startRun: (lensId, request = {}) => sent(client.POST("/lens/{lens_id}/runs", { ...lens(lensId), body: request })),
    watchAll: () => required(client.POST("/lens/watch-all", { headers })),
    signalConfig: () => required(client.GET("/lens/admin/signals", { headers })),
    signalOverview: () => apiClient.get("/lens/admin/signals/overview", { accessToken }),
    alertRules: () => apiClient.get("/lens/alerts/rules", { accessToken }),
    saveAlertRules: (body) => apiClient.put("/lens/alerts/rules", { accessToken, body }),
    alertRuleOverview: () => apiClient.get("/lens/alerts/rules/overview", { accessToken }),
    saveSignalConfig: (config) => required(client.PUT("/lens/admin/signals", { headers, body: config })),
    signalAlerts: () => apiClient.get("/lens/admin/signals/alerts", { accessToken }),
    saveSignalAlerts: (body) => apiClient.put("/lens/admin/signals/alerts", { accessToken, body }),
    testSignalAlerts: (body) => apiClient.post("/lens/admin/signals/alerts/test", { accessToken, body }),
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
