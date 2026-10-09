// Mirrors the eval types in schema/lens.v1.json, generated from the lens-contract Rust crate
export type RunStatus = "running" | "scoring" | "done" | "failed";

export interface CaseDiff {
  readonly case_id: string;
  readonly title: string;
  readonly critical: boolean;
  readonly baseline_url: string;
  readonly candidate_url: string;
}

export interface GateResult {
  readonly passed: boolean;
  readonly reasons: readonly string[];
}

export interface Summary {
  readonly passed: number;
  readonly total: number;
  readonly pass_rate: number;
  readonly cost_per_case: number;
  readonly scores: Readonly<Record<string, number>>;
  readonly errors: number;
  readonly baseline_run_id: string | null;
  readonly baseline_version: string | null;
  readonly regressions: readonly CaseDiff[];
  readonly fixed: readonly CaseDiff[];
  readonly gate: GateResult;
}

export interface EvalRun {
  readonly id: string;
  readonly status: RunStatus;
  readonly eval: string;
  readonly agent: string;
  readonly version: string;
  readonly branch: string;
  readonly pr: number | null;
  readonly url: string;
  readonly expected_trials: number;
  readonly received_trials: number;
  readonly summary: Summary | null;
  readonly failure: string;
  readonly ci_url?: string;
}

export interface ToolStep {
  readonly name: string;
  readonly tool_name: string;
  readonly ok: boolean;
  readonly start_ns: number;
  readonly end_ns: number;
}

export interface ScorerCheck {
  readonly scorer: string;
  readonly passed: boolean;
}

export interface TrialTrace {
  readonly trace_id: string;
  readonly trace_ref: string;
}

export interface TrialSteps {
  readonly traces?: readonly TrialTrace[];
  readonly output?: string | null;
  readonly trial: number;
  readonly error: string | null;
  readonly checks: readonly ScorerCheck[];
  readonly steps: readonly ToolStep[];
}

export interface RunCaseSummary {
  readonly case_id: string;
  readonly title: string;
  readonly critical: boolean;
  readonly passed: boolean | null;
}

export interface RunCase extends RunCaseSummary {
  readonly trials: readonly TrialSteps[];
}

export type Scorer =
  | { readonly kind: "task_completed" }
  | { readonly kind: "called_before"; readonly first: string; readonly then: string }
  | { readonly kind: "judge"; readonly prompt: string; readonly model?: string };

export interface Gate {
  readonly regressions?: number | null;
  readonly critical?: number | null;
  readonly pass_rate?: number | null;
  readonly cost_per_case?: number | null;
  readonly min?: Readonly<Record<string, number>>;
}

export interface EvalSpec {
  readonly agent: string;
  readonly dataset_id: string;
  readonly revision?: number | null;
  readonly scorers: readonly Scorer[];
  readonly trials?: number;
  readonly baseline?: string;
  readonly gate?: Gate;
  readonly timeout_per_trial_ms?: number;
}

export interface EvalDefinition {
  readonly name: string;
  readonly spec: Required<Omit<EvalSpec, "revision">> & {
    readonly revision: number | null;
    readonly agent_io?: { readonly version: number; readonly connection: string } | null;
  };
  readonly updated_at: string;
}

export interface EvalRunFilter {
  readonly include_ci?: boolean;
  readonly eval?: string;
  readonly agent?: string;
  readonly branch?: string;
  readonly dataset?: string;
  readonly cursor?: string;
  readonly limit?: number;
}
