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

export interface TrialSteps {
  readonly trial: number;
  readonly error: string | null;
  readonly checks: readonly ScorerCheck[];
  readonly steps: readonly ToolStep[];
}

export interface RunCase {
  readonly case_id: string;
  readonly title: string;
  readonly critical: boolean;
  readonly passed: boolean | null;
  readonly trials: readonly TrialSteps[];
}

export interface EvalRunFilter {
  readonly eval?: string;
  readonly agent?: string;
  readonly branch?: string;
  readonly dataset?: string;
  readonly cursor?: string;
  readonly limit?: number;
}
