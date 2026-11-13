import { z } from "zod";
import { apiClient } from "../../../lib/http/requests";
import type { OptimizationStore } from "../data/demo/demoOptimizations";
import { configSchema, optimizationSchema, type Optimization, type OptimizationState } from "./model";

const runSchema = z.object({
  id: z.string(), config: configSchema, status: optimizationSchema.shape.status,
  createdAt: z.number(), results: optimizationSchema.shape.measured.unwrap(), pending: z.number(), error: z.string().nullable(),
});
const modelSchema = z.object({ data: z.array(z.object({
  model_group: z.string(), input_cost_per_token: z.number().nullable(), output_cost_per_token: z.number().nullable(),
})) });

function runView(run: z.infer<typeof runSchema>): Optimization {
  const sources = [...new Set(run.results.map((result) => result.sourceId))];
  return {
    id: run.id, config: run.config, status: run.status, createdAt: run.createdAt,
    seed: false, completed: run.results.length, target: run.results.length + run.pending,
    measured: run.results, error: run.error,
    examples: sources.map((id) => {
      const results = run.results.filter((result) => result.sourceId === id);
      const source = results[0];
      return {
        id, question: source.input, original: source.original, candidate: "", context: source.input,
        traceId: source.traceId, spanId: source.spanId,
        results: Object.fromEntries(results.map((result) => [result.model, {output: result.output, score: result.score, error: result.error ?? result.scoreError}])),
      };
    }),
  };
}

function tier(id: string) {
  if (/(astra|fable|carbon|wonder)/i.test(id)) return "Frontier";
  if (/(flash|haiku|luna|mini|nano)/i.test(id)) return "Flash";
  if (/(sol|opus|sonnet|argon|spark)/i.test(id)) return "Balanced";
  return "Other";
}

export function liveOptimizationStore(accessToken: string): OptimizationStore {
  let state: OptimizationState = { version: 1, runs: [], connectedAgents: [], loading: true, models: [], agents: [] };
  const listeners = new Set<() => void>();
  let pending = false;
  const publish = (next: OptimizationState) => { state = next; listeners.forEach((listener) => listener()); };
  const refresh = async () => {
    if (pending) return;
    pending = true;
    try {
      const data = await apiClient.get<unknown>("/lens/optimizations", { accessToken });
      const runs = z.array(runSchema).parse(data).map(runView);
      if (state.loading || !state.models?.length) {
        const [catalog, agents] = await Promise.all([
          apiClient.get<unknown>("/lens/optimizations/models", { accessToken }),
          apiClient.get<unknown>("/v1/traces/agents", { accessToken }),
        ]);
        const models = modelSchema.parse(catalog).data.map((model) => ({
          id: model.model_group, tier: tier(model.model_group),
          cost: model.input_cost_per_token !== null && model.output_cost_per_token !== null ? model.input_cost_per_token + model.output_cost_per_token : null,
        }));
        const names = z.object({agents: z.array(z.object({name: z.string()}))}).parse(agents).agents.map((agent) => agent.name);
        publish({...state, runs, models, agents: names, loading: false, error: undefined});
      } else publish({...state, runs, loading: false, error: undefined});
    } catch (error) {
      publish({...state, loading: false, error: error instanceof Error ? error.message : "Could not load optimizations"});
    } finally { pending = false; }
  };
  return {
    live: true,
    subscribe: (listener) => { listeners.add(listener); return () => { listeners.delete(listener); }; },
    snapshot: () => state,
    advance: () => { void refresh(); },
    create: async (config) => {
      const created = runSchema.parse(await apiClient.post<unknown>("/lens/optimizations", { accessToken, body: config }));
      publish({...state, runs: [runView(created), ...state.runs]});
      return created.id;
    },
    setStatus: (id, status) => {
      void apiClient.post<unknown>(`/lens/optimizations/${encodeURIComponent(id)}/status`, {accessToken, body:{status}})
        .then(() => refresh()).catch((error: unknown) => publish({...state, error: error instanceof Error ? error.message : "Could not update optimization"}));
    },
    connect: () => {},
    remove: () => {},
  };
}
