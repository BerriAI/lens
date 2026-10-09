export type Verdict = "pass" | "fail" | "error";
export type RunStatus = "running" | "finished" | "error";

export interface CaseOutcome {
  readonly verdict: Verdict;
  readonly trace_id: string;
  readonly trace_ref: string;
  readonly trace_count?: number;
}

export interface CaseDiff {
  readonly case_id: string;
  readonly input: string;
  readonly critical: boolean;
  readonly baseline: CaseOutcome | null;
  readonly candidate: CaseOutcome;
}

export interface GateResult {
  readonly passed: boolean;
  readonly reasons: readonly string[];
}

export interface Summary {
  readonly total: number;
  readonly passed: number;
  readonly failed: number;
  readonly errored: number;
  readonly pass_rate: number;
  readonly cost_usd: number | null;
  readonly baseline_run_id: string | null;
  readonly baseline_reason: string | null;
  readonly pass_rate_delta: number | null;
  readonly regressions: readonly CaseDiff[];
  readonly fixed: readonly CaseDiff[];
}

export interface EvalRun {
  readonly id: string;
  readonly eval: string;
  readonly agent: string;
  readonly dataset_id: string;
  readonly dataset_revision: number;
  readonly branch: string;
  readonly commit_sha: string;
  readonly pr_url: string | null;
  readonly pr_number?: number | null;
  readonly failure?: string;
  readonly status: RunStatus;
  readonly created_at: string;
  readonly finished_at: string | null;
  readonly summary: Summary | null;
  readonly gate: GateResult | null;
  readonly cases?: readonly CaseDiff[];
}

export interface EvalRunFilter {
  readonly eval?: string;
  readonly agent?: string;
  readonly branch?: string;
}
