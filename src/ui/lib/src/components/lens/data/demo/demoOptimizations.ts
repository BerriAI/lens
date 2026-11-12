import type { DatasetsApi } from "../../datasets/client";
import {
  baselineFor,
  catalog,
  configSchema,
  defaultConfig,
  demoDatasetNames,
  modelEligibility,
  optimizationStateSchema,
  type ComparisonExample,
  type Optimization,
  type OptimizationConfig,
  type OptimizationState,
} from "../../optimizations/model";
import type { LensDemoData } from "./fixtures";
import { readDemoState, writeDemoState, type DemoStorage } from "./storage";

export interface OptimizationStore {
  subscribe(listener: () => void): () => void;
  snapshot(): OptimizationState;
  create(config: OptimizationConfig): Promise<string>;
  setStatus(id: string, status: Optimization["status"]): void;
  advance(): void;
  connect(agent: string): void;
  remove(id: string): void;
}
export function demoDatasetId(agent: string) {
  return `demo-${agent}-dataset`;
}
export function optimizationRuns(data: LensDemoData, agent: string) {
  return data.runs
    .filter((run) => run.trace.summary.agent_names?.includes(agent))
    .toSorted((a, b) => Number(a.trace.summary.status === "error") - Number(b.trace.summary.status === "error"));
}
function traceExamples(data: LensDemoData, agent: string): ComparisonExample[] {
  return optimizationRuns(data, agent).map(({ trace, details }) => {
    const step = trace.spans.findLast((span) => span.type === "llm");
    const detail = details.find((span) => span.span_id === step?.span_id);
    const preceding = details.filter((detail) => {
      const span = trace.spans.find((item) => item.span_id === detail.span_id);
      return (
        span &&
        span.type !== "agent" &&
        span.span_id !== step?.span_id &&
        span.start_offset_ms <= (step?.start_offset_ms ?? Infinity)
      );
    });
    return {
      id: trace.summary.trace_id,
      question: trace.summary.input_preview,
      context:
        preceding
          .map(
            (detail) =>
              `${trace.spans.find((span) => span.span_id === detail.span_id)?.name}\n${detail.input}\n${detail.output}`,
          )
          .join("\n\n") || "Recorded conversation",
      original: detail?.output ?? "",
      candidate: detail?.output ?? "",
      traceId: trace.summary.trace_id,
      spanId: step?.span_id ?? "",
    };
  });
}
export function createOptimizationStore(
  data: LensDemoData,
  datasets: DatasetsApi,
  storage?: DemoStorage,
  now = Date.now(),
): OptimizationStore {
  const seeds: Optimization[] = (
    [
      {
        mode: "live",
        judge: "feedback",
        id: "demo-live",
        agent: "support_agent",
        models: ["gpt-6.1-sol", "claude-opus-5-5", "gpt-6-luna"],
      },
      {
        mode: "live",
        judge: "llm",
        id: "demo-live-judge",
        agent: "sales_agent",
        models: ["gpt-6.1-sol", "gemini-4-argon", "claude-haiku-5-5"],
      },
      {
        mode: "shadow",
        judge: "llm",
        id: "demo-shadow",
        agent: "research_agent",
        models: ["claude-opus-5-5", "gemini-4-carbon", "gpt-6-luna"],
      },
      {
        mode: "offline",
        judge: "llm",
        id: "demo-offline",
        agent: "release_agent",
        models: ["gpt-6.1-sol", "claude-fable-5-2", "gpt-6-luna"],
      },
    ] as const
  ).map(({ mode, judge, id, agent, models }) => {
    const examples = traceExamples(data, agent);
    const target = mode === "offline" ? examples.length : mode === "live" ? 4800 : 1240;
    return {
      id,
      config: {
        ...defaultConfig(mode, agent),
        judge,
        models: [...models],
        datasetId: demoDatasetId(agent),
        datasetName: demoDatasetNames[agent],
      },
      status: "completed",
      createdAt: now - 7 * 86400_000,
      completed: target,
      target,
      seed: true,
      examples,
    };
  });
  const saved = optimizationStateSchema.safeParse(readDemoState(storage, "optimizations"));
  let state: OptimizationState =
    saved.success &&
    saved.data.runs.every((run) => run.config.models.every((id) => catalog.some((model) => model.id === id)))
      ? saved.data
      : { version: 1, runs: seeds, connectedAgents: [] };
  const listeners = new Set<() => void>();
  const publish = (next: OptimizationState) => {
    state = next;
    writeDemoState(storage, "optimizations", state);
    listeners.forEach((listener) => listener());
  };
  return {
    subscribe: (listener) => {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    snapshot: () => state,
    create: async (config) => {
      configSchema.parse(config);
      if (
        !config.models.length ||
        config.models.some(
          (id) =>
            !catalog.some((model) => model.id === id) ||
            modelEligibility(config.agent, id) ||
            id === baselineFor(config.agent),
        )
      )
        throw new Error("Choose an available candidate model.");
      if (config.mode === "offline" && config.scope === "agent" && !state.connectedAgents.includes(config.agent))
        throw new Error("Connect this agent before running the comparison.");
      let examples = traceExamples(data, config.agent);
      if (config.mode === "offline") {
        const dataset = await datasets.get(config.datasetId, config.revision);
        examples = dataset.cases
          .filter((item) => item.included)
          .map((item) => ({
            id: item.id,
            question: item.messages.findLast((m) => m.role === "user")?.content ?? "Saved conversation",
            context: item.messages
              .filter((m) => m.role !== "user")
              .map((m) => m.content)
              .join("\n"),
            original: item.reply,
            candidate: item.expected || item.reply,
            traceId: item.source.trace_id,
            spanId: item.source.span_id,
          }));
      }
      if (!examples.length) throw new Error("This dataset has no included cases.");
      const id = `optimization-${crypto.randomUUID()}`;
      publish({
        ...state,
        runs: [
          {
            id,
            config: {
              ...config,
              judge: config.mode === "live" ? config.judge : "llm",
            },
            status: "running",
            createdAt: Date.now(),
            completed: 0,
            target: config.mode === "offline" ? examples.length : 600,
            seed: false,
            examples,
          },
          ...state.runs,
        ],
      });
      return id;
    },
    setStatus: (id, status) =>
      publish({
        ...state,
        runs: state.runs.map((run) => (run.id === id ? { ...run, status } : run)),
      }),
    connect: (agent) =>
      publish({
        ...state,
        connectedAgents: [...new Set([...state.connectedAgents, agent])],
      }),
    remove: (id) => publish({ ...state, runs: state.runs.filter((run) => run.id !== id) }),
    advance: () => {
      if (!state.runs.some((run) => run.status === "running")) return;
      publish({
        ...state,
        runs: state.runs.map((run) => {
          if (run.status !== "running") return run;
          const completed =
            run.config.mode === "offline"
              ? Math.min(run.target, run.completed + Math.max(1, Math.ceil(run.target / 8)))
              : run.completed + 75;
          return {
            ...run,
            completed,
            status: run.config.mode === "offline" && completed === run.target ? "completed" : run.status,
          };
        }),
      });
    },
  };
}
