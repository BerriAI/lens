import { z } from "zod";
import type { DatasetsApi } from "../../datasets/client";
import type { EvalRunsApi } from "../../evals/runs/client";
import type { EvalDefinition, EvalRun, RunCase } from "../../evals/runs/types";
import { demoDatasetId } from "./demoOptimizations";
import { readDemoState, writeDemoState, type DemoStorage } from "./storage";

const scorer = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("task_completed") }),
  z.object({
    kind: z.literal("called_before"),
    first: z.string(),
    then: z.string(),
  }),
  z.object({
    kind: z.literal("judge"),
    prompt: z.string(),
    model: z.string().optional(),
  }),
]);
const definitionSchema = z.object({
  name: z.string(),
  updated_at: z.string(),
  spec: z.object({
    agent: z.string(),
    dataset_id: z.string(),
    revision: z.number().nullable(),
    scorers: z.array(scorer),
    trials: z.number(),
    baseline: z.string(),
    timeout_per_trial_ms: z.number(),
    gate: z.object({
      regressions: z.number().nullable().optional(),
      critical: z.number().nullable().optional(),
      pass_rate: z.number().nullable().optional(),
      cost_per_case: z.number().nullable().optional(),
      min: z.record(z.string(), z.number()).optional(),
    }),
  }),
});

export function demoEvalsApi(datasets: DatasetsApi, storage?: DemoStorage): EvalRunsApi {
  const saved = z.array(definitionSchema).safeParse(readDemoState(storage, "evals"));
  let definitions: EvalDefinition[] = saved.success
    ? saved.data
    : [
        {
          name: "support-quality",
          updated_at: new Date().toISOString(),
          spec: {
            agent: "support_agent",
            dataset_id: demoDatasetId("support_agent"),
            revision: 1,
            scorers: [{ kind: "task_completed" }],
            trials: 1,
            baseline: "main",
            timeout_per_trial_ms: 60000,
            gate: { pass_rate: 0.9 },
          },
        },
      ];
  let runs: EvalRun[] = [];
  const results = new Map<string, RunCase[]>();
  const definition = (name: string) => {
    const value = definitions.find((item) => item.name === name);
    if (!value) throw new Error("Eval not found in this demo.");
    return value;
  };
  const execute = async (name: string) => {
    const current = definition(name);
    const dataset = await datasets.get(current.spec.dataset_id, current.spec.revision ?? undefined);
    const cases: RunCase[] = dataset.cases
      .filter((item) => item.included)
      .map((item) => ({
        case_id: item.id,
        title: item.messages.findLast((message) => message.role === "user")?.content ?? "Saved conversation",
        critical: false,
        passed: true,
        trials: Array.from({ length: current.spec.trials }, (_, trial) => ({
          trial,
          error: null,
          output: item.expected || item.reply,
          traces: item.source.trace_id
            ? [
                {
                  trace_id: item.source.trace_id,
                  trace_ref: item.source.trace_ref,
                },
              ]
            : [],
          checks: current.spec.scorers.map((scorer) => ({
            scorer: scorer.kind,
            passed: true,
          })),
          steps: [],
        })),
      }));
    const id = `demo-eval-${crypto.randomUUID()}`;
    const run: EvalRun = {
      id,
      eval: name,
      agent: current.spec.agent,
      status: "done",
      version: "demo-v1",
      branch: "main",
      pr: null,
      url: "",
      expected_trials: cases.length * current.spec.trials,
      received_trials: cases.length * current.spec.trials,
      failure: "",
      summary: {
        passed: cases.length,
        total: cases.length,
        pass_rate: 1,
        cost_per_case: 0.024,
        scores: { task_completed: 1 },
        errors: 0,
        baseline_run_id: runs.find((run) => run.eval === name)?.id ?? null,
        baseline_version: "demo-v1",
        regressions: [],
        fixed: [],
        gate: { passed: true, reasons: [] },
      },
    };
    results.set(id, cases);
    runs = [run, ...runs];
  };
  const ready = Promise.all(definitions.map((item) => execute(item.name)));
  return {
    evals: async () => definitions,
    evalDefinition: async (name) => definition(name),
    saveEval: async (name, spec) => {
      const value: EvalDefinition = {
        name,
        updated_at: new Date().toISOString(),
        spec: {
          ...spec,
          revision: spec.revision ?? null,
          trials: spec.trials ?? 1,
          baseline: spec.baseline ?? "main",
          gate: spec.gate ?? {},
          timeout_per_trial_ms: spec.timeout_per_trial_ms ?? 60000,
        },
      };
      definitions = [...definitions.filter((item) => item.name !== name), value];
      writeDemoState(storage, "evals", definitions);
      return value;
    },
    list: async (filter) => {
      await ready;
      return runs.filter(
        (run) => (!filter.eval || filter.eval === run.eval) && (!filter.agent || filter.agent === run.agent),
      );
    },
    get: async (id) => {
      await ready;
      const run = runs.find((item) => item.id === id);
      if (!run) throw new Error("Run not found.");
      return run;
    },
    cases: async (id) => {
      await ready;
      return results.get(id) ?? [];
    },
    runCase: async (id, caseId) => {
      await ready;
      const item = results.get(id)?.find((item) => item.case_id === caseId);
      if (!item) throw new Error("Case not found.");
      return item;
    },
    runDemo: execute,
  };
}
