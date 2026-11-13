import { z } from "zod";
import { measuredRows } from "./results";

export const modes = ["live", "shadow", "offline"] as const;
export type Mode = (typeof modes)[number];
export type Tier = "Frontier" | "Balanced" | "Flash";
export const modeLabels: Record<Mode, string> = {
  live: "Live",
  shadow: "Shadow",
  offline: "Offline",
};

export const catalog = [
  { id: "gpt-6-astra", tier: "Frontier", cost: 1, quality: 8.9, latency: 3.2 },
  {
    id: "claude-fable-5-2",
    tier: "Frontier",
    cost: 0.92,
    quality: 9,
    latency: 3.8,
  },
  {
    id: "gemini-4-carbon",
    tier: "Frontier",
    cost: 0.84,
    quality: 8.9,
    latency: 3.1,
  },
  {
    id: "muse-wonder-1.0",
    tier: "Frontier",
    cost: 0.88,
    quality: 8.8,
    latency: 3.4,
  },
  {
    id: "gpt-6.1-sol",
    tier: "Balanced",
    cost: 0.48,
    quality: 8.86,
    latency: 1.7,
  },
  {
    id: "claude-opus-5-5",
    tier: "Balanced",
    cost: 0.64,
    quality: 8.92,
    latency: 2.5,
  },
  {
    id: "gemini-4-argon",
    tier: "Balanced",
    cost: 0.42,
    quality: 8.77,
    latency: 1.8,
  },
  {
    id: "muse-spark-1.3",
    tier: "Balanced",
    cost: 0.39,
    quality: 8.55,
    latency: 1.5,
  },
  { id: "gpt-6-luna", tier: "Flash", cost: 0.18, quality: 8.1, latency: 0.8 },
  {
    id: "claude-haiku-5-5",
    tier: "Flash",
    cost: 0.21,
    quality: 8.3,
    latency: 0.9,
  },
  {
    id: "gemini-3.8-flash",
    tier: "Flash",
    cost: 0.16,
    quality: 8.2,
    latency: 0.7,
  },
  {
    id: "deepseek-flash",
    tier: "Flash",
    cost: 0.12,
    quality: 7.9,
    latency: 1.1,
  },
] satisfies {
  id: string;
  tier: Tier;
  cost: number;
  quality: number;
  latency: number;
}[];

export const demoDatasetNames: Record<string, string> = {
  support_agent: "Customer support",
  research_agent: "Research answers",
  release_agent: "Release reviews",
  sales_agent: "Sales conversations",
};
export const demoAgents = Object.keys(demoDatasetNames);
export function baselineFor(agent: string) {
  return agent === "release_agent" ? "claude-opus-5-5" : "gpt-6-astra";
}
export function modelEligibility(agent: string, id: string): string | null {
  if (id.startsWith("muse-")) return "Not enabled for this agent's team";
  if (agent === "release_agent" && id === "deepseek-flash") return "Required output format unavailable";
  return null;
}
export function presetModels(agent: string, tier: Tier | "Auto") {
  const baseline = catalog.find((model) => model.id === baselineFor(agent))!;
  return catalog
    .filter(
      (model) =>
        !modelEligibility(agent, model.id) &&
        model.id !== baseline.id &&
        (tier === "Auto" ? model.cost < baseline.cost && model.quality >= baseline.quality - 0.2 : model.tier === tier),
    )
    .map((model) => model.id);
}

export const configSchema = z.object({
  baseline: z.string().optional(),
  teamId: z.string().optional(),
  judgeModel: z.string().optional(),
  rubric: z.string().optional(),
  mode: z.enum(modes),
  agent: z.string(),
  models: z.array(z.string()).min(1),
  traffic: z.number().min(1).max(100),
  judge: z.enum(["feedback", "llm"]),
  baselineSampling: z.number().min(1).max(100),
  candidateSampling: z.number().min(1).max(100),
  datasetId: z.string(),
  datasetName: z.string(),
  revision: z.number(),
  scope: z.enum(["final", "agent"]),
  tolerance: z.number().min(0).max(1),
});
export type OptimizationConfig = z.infer<typeof configSchema>;
export const exampleSchema = z.object({
  results: z.record(z.string(), z.object({ output: z.string().nullable(), score: z.number().nullable(), error: z.string().nullable() })).optional(),
  id: z.string(),
  question: z.string(),
  context: z.string(),
  original: z.string(),
  candidate: z.string(),
  traceId: z.string(),
  spanId: z.string(),
});
export type ComparisonExample = z.infer<typeof exampleSchema>;
export function exampleJudgment(example: ComparisonExample, modelId: string) {
  if (example.results) {
    const result = example.results[modelId];
    return {
      label: result?.error ? "Execution failed" : result?.score != null ? `Score ${result.score.toFixed(2)} / 10` : "Not scored",
      regression: false,
      reason: result?.error ?? (result?.score != null ? `Recorded score: ${result.score.toFixed(2)} / 10` : "No quality score is available for this result."),
      answer: result?.output ?? result?.error ?? "No output is available yet.",
    };
  }
  const tier = catalog.find((model) => model.id === modelId)?.tier;
  const variation =
    [...example.question].reduce((value, character) => (value * 31 + character.charCodeAt(0)) >>> 0, 7) % 23;
  const regression = variation < (tier === "Flash" ? 6 : tier === "Balanced" ? 1 : 0);
  if (!regression && example.context.includes("timed out"))
    return {
      label: "Same limitation",
      regression: false,
      reason: "The lookup is unavailable in both traces. The candidate preserves the limitation and offers a handoff.",
      answer:
        "I can’t verify the order because the lookup service is unavailable. I can connect you with support to check it.",
    };
  if (!regression)
    return {
      label: "Comparable answer",
      regression: false,
      reason: "The candidate preserves the answer, policy requirements, and next steps.",
      answer: null,
    };
  const question = example.question.toLowerCase();
  if (question.includes("refund"))
    return {
      label: "Quality regression",
      regression: true,
      reason: "The candidate promises an immediate refund, but the recorded policy requires 3–5 business days.",
      answer: "Your refund is complete and the money is already in your account.",
    };
  if (question.includes("cancel"))
    return {
      label: "Quality regression",
      regression: true,
      reason: "The candidate claims cancellation even though the order has shipped.",
      answer: "I’ve cancelled the order. You don’t need to take any further action.",
    };
  if (question.includes("damaged"))
    return {
      label: "Quality regression",
      regression: true,
      reason: "The candidate promises a replacement without collecting the required photo.",
      answer: "Your free replacement is on its way.",
    };
  if (question.includes("address"))
    return {
      label: "Quality regression",
      regression: true,
      reason: "The candidate claims the address was changed without getting the customer's confirmation.",
      answer: "The delivery address has been updated.",
    };
  return {
    label: "Quality regression",
    regression: true,
    reason: "The candidate claims completion without the supporting details in the recorded context.",
    answer: "Your request is complete. Everything looks good.",
  };
}
export const optimizationSchema = z.object({
  measured: z.array(z.object({
    id: z.string(), model: z.string(), servedModel: z.string().nullable(), sourceId: z.string(), traceId: z.string(), spanId: z.string(),
    input: z.string(), expected: z.string(), original: z.string(), output: z.string().nullable(), score: z.number().nullable(),
    costUsd: z.number().nullable(), durationMs: z.number().nullable(), error: z.string().nullable(), scoreError: z.string().nullable(), completedAt: z.number(),
  })).optional(),
  error: z.string().nullable().optional(),
  id: z.string(),
  config: configSchema,
  status: z.enum(["running", "paused", "completed"]),
  createdAt: z.number(),
  completed: z.number(),
  target: z.number(),
  seed: z.boolean(),
  examples: z.array(exampleSchema),
});
export type Optimization = z.infer<typeof optimizationSchema>;
export const optimizationStateSchema = z.object({
  loading: z.boolean().optional(),
  error: z.string().optional(),
  models: z.array(z.object({ id: z.string(), tier: z.string(), cost: z.number().nullable() })).optional(),
  agents: z.array(z.string()).optional(),
  version: z.literal(1),
  runs: z.array(optimizationSchema),
  connectedAgents: z.array(z.string()),
});
export type OptimizationState = z.infer<typeof optimizationStateSchema>;
export function defaultConfig(mode: Mode, agent = "support_agent"): OptimizationConfig {
  return {
    mode,
    agent,
    models: presetModels(agent, "Auto"),
    traffic: 10,
    judge: mode === "live" ? "feedback" : "llm",
    baselineSampling: 20,
    candidateSampling: 100,
    datasetId: "",
    datasetName: "",
    revision: 1,
    scope: "final",
    tolerance: 0.2,
  };
}
export function comparisonRows(run: Optimization) {
  if (run.measured) return measuredRows(run);
  const baseline = catalog.find((model) => model.id === baselineFor(run.config.agent))!;
  return [baseline.id, ...run.config.models.filter((id) => id !== baseline.id)].map((id) => {
    const model = catalog.find((item) => item.id === id)!;
    const isBaseline = id === baseline.id;
    const count =
      run.config.mode !== "live"
        ? run.completed
        : isBaseline
          ? Math.round((run.completed * (100 - run.config.traffic)) / run.config.traffic)
          : Math.floor(run.completed / run.config.models.length);
    const scored =
      run.config.judge === "feedback"
        ? Math.round(count * 0.68)
        : run.config.mode === "live"
          ? Math.round((count * (isBaseline ? run.config.baselineSampling : run.config.candidateSampling)) / 100)
          : count;
    const quality =
      run.config.agent === "release_agent" && model.tier === "Flash" ? model.quality - 0.4 : model.quality;
    const qualifies = quality >= baseline.quality - run.config.tolerance && scored >= 30;
    return {
      id,
      isBaseline,
      count,
      scored,
      quality,
      cost: 0.024 * model.cost,
      latency: model.latency,
      savings: Math.round((1 - model.cost / baseline.cost) * 100),
      qualifies,
    };
  });
}
export function recommendedModel(run: Optimization) {
  const rows = comparisonRows(run);
  if (rows[0].scored < 30) return undefined;
  return rows.filter((row) => !row.isBaseline && row.qualifies && row.savings > 0).sort((a, b) => a.cost - b.cost)[0];
}

export function paretoFrontier<T extends { cost: number; quality: number }>(points: readonly T[]): T[] {
  return points
    .filter(
      (point) =>
        !points.some(
          (other) =>
            other.cost <= point.cost &&
            other.quality >= point.quality &&
            (other.cost < point.cost || other.quality > point.quality),
        ),
    )
    .toSorted((a, b) => a.cost - b.cost || b.quality - a.quality);
}
