import type { EvalSpec, Gate, Scorer } from "./runs/types";

export interface NewEvalDraft {
  readonly name: string;
  readonly datasetId: string;
  readonly agent: string;
  readonly taskCompleted: boolean;
  readonly calledBefore: boolean;
  readonly first: string;
  readonly then: string;
  readonly judge: boolean;
  readonly prompt: string;
  readonly trials: string;
  readonly regressions: string;
  readonly critical: string;
  readonly passRate: string;
  readonly costPerCase: string;
}

export const EMPTY_DRAFT: NewEvalDraft = {
  name: "",
  datasetId: "",
  agent: "",
  taskCompleted: true,
  calledBefore: false,
  first: "",
  then: "",
  judge: false,
  prompt: "",
  trials: "3",
  regressions: "0",
  critical: "0",
  passRate: "",
  costPerCase: "",
};

export type DraftResult =
  | { readonly ok: true; readonly name: string; readonly spec: EvalSpec }
  | { readonly ok: false; readonly error: string };

const NAME = /^[a-z0-9][a-z0-9_-]*$/;

function optionalNumber(value: string): number | null | undefined {
  if (value.trim() === "") return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : undefined;
}

export function buildEvalSpec(draft: NewEvalDraft): DraftResult {
  const name = draft.name.trim();
  if (!NAME.test(name) || name === "runs")
    return { ok: false, error: "Name must be lowercase letters, digits, - or _" };
  if (!draft.datasetId) return { ok: false, error: "Pick a dataset" };
  if (!draft.agent.trim()) return { ok: false, error: "Agent is required" };
  if (draft.calledBefore && (!draft.first.trim() || !draft.then.trim()))
    return { ok: false, error: "Name both tools for the ordering check" };
  if (draft.judge && !draft.prompt.trim()) return { ok: false, error: "The judge needs a question" };
  const scorers: Scorer[] = [
    ...(draft.taskCompleted ? [{ kind: "task_completed" } as const] : []),
    ...(draft.calledBefore
      ? [{ kind: "called_before", first: draft.first.trim(), then: draft.then.trim() } as const]
      : []),
    ...(draft.judge ? [{ kind: "judge", prompt: draft.prompt.trim(), model: "" } as const] : []),
  ];
  if (scorers.length === 0) return { ok: false, error: "Pick at least one scorer" };
  const trials = Number(draft.trials);
  if (!Number.isSafeInteger(trials) || trials < 1)
    return { ok: false, error: "Trials must be a positive whole number" };
  const regressions = optionalNumber(draft.regressions);
  const critical = optionalNumber(draft.critical);
  const passRate = optionalNumber(draft.passRate);
  const costPerCase = optionalNumber(draft.costPerCase);
  if ([regressions, critical, passRate, costPerCase].includes(undefined) || (passRate ?? 0) > 100)
    return { ok: false, error: "Gate limits must be non-negative numbers, pass rate at most 100" };
  const gate: Gate = {
    regressions: regressions === null || regressions === undefined ? null : Math.floor(regressions),
    critical: critical === null || critical === undefined ? null : Math.floor(critical),
    pass_rate: passRate === null || passRate === undefined ? null : passRate / 100,
    cost_per_case: costPerCase ?? null,
    min: {},
  };
  return {
    ok: true,
    name,
    spec: { agent: draft.agent.trim(), dataset_id: draft.datasetId, scorers, trials, baseline: "main", gate },
  };
}
