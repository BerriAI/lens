import type { CaseDiff, CaseOutcome, EvalRun } from "./types";

interface ContractDiff {
  readonly case_id: string;
  readonly title: string;
  readonly critical: boolean;
  readonly baseline_url: string;
  readonly candidate_url: string;
}

interface ContractSummary {
  readonly passed: number;
  readonly total: number;
  readonly pass_rate: number;
  readonly cost_per_case: number;
  readonly errors: number;
  readonly scores: Readonly<Record<string, number>>;
  readonly baseline_run_id: string | null;
  readonly baseline_version: string | null;
  readonly regressions: readonly ContractDiff[];
  readonly fixed: readonly ContractDiff[];
  readonly gate: {
    readonly passed: boolean;
    readonly reasons: readonly string[];
  };
}

interface ContractRun {
  readonly id: string;
  readonly status: "running" | "scoring" | "done" | "failed";
  readonly eval: string;
  readonly agent: string;
  readonly version: string;
  readonly branch: string;
  readonly pr: number | null;
  readonly url: string;
  readonly expected_trials: number;
  readonly received_trials: number;
  readonly summary: ContractSummary | null;
  readonly failure: string;
}

interface CaseDetails {
  readonly case_id: string;
  readonly input: string;
  readonly verdict: boolean | null;
  readonly traces: readonly {
    readonly trace_id: string;
    readonly trace_ref: string;
  }[];
}

export interface RunDetails {
  readonly run: ContractRun;
  readonly dataset_id: string;
  readonly dataset_revision: number;
  readonly created_at: string;
  readonly completed_at: string | null;
  readonly ci_url: string;
  readonly cases: readonly CaseDetails[];
  readonly baseline: {
    readonly run: ContractRun;
    readonly cases: readonly CaseDetails[];
  } | null;
}

const outcome = (record: CaseDetails | undefined): CaseOutcome => ({
  verdict: record?.verdict == null ? "error" : record.verdict ? "pass" : "fail",
  trace_id: record?.traces[0]?.trace_id ?? "",
  trace_ref: record?.traces[0]?.trace_ref ?? "",
  trace_count: record?.traces.length ?? 0,
});

const diff = (details: RunDetails, item: ContractDiff): CaseDiff => {
  const candidate = details.cases.find(
    (value) => value.case_id === item.case_id,
  );
  const baseline = details.baseline?.cases.find(
    (value) => value.case_id === item.case_id,
  );
  return {
    case_id: item.case_id,
    input: candidate?.input ?? item.title,
    critical: item.critical,
    baseline: details.baseline ? outcome(baseline) : null,
    candidate: outcome(candidate),
  };
};

export function evalRun(details: RunDetails): EvalRun {
  const { run } = details;
  const summary = run.summary;
  const baseline = details.baseline?.run.summary;
  return {
    id: run.id,
    eval: run.eval,
    agent: run.agent,
    dataset_id: details.dataset_id,
    dataset_revision: details.dataset_revision,
    branch: run.branch,
    commit_sha: run.version,
    pr_url: null,
    pr_number: run.pr,
    status:
      run.status === "done"
        ? "finished"
        : run.status === "failed"
          ? "error"
          : "running",
    failure: run.failure,
    created_at: details.created_at,
    finished_at: details.completed_at,
    gate: summary?.gate ?? null,
    cases: details.cases.map((record) => ({
      case_id: record.case_id,
      input: record.input,
      critical: false,
      baseline: details.baseline
        ? outcome(
            details.baseline.cases.find(
              (item) => item.case_id === record.case_id,
            ),
          )
        : null,
      candidate: outcome(record),
    })),
    summary: summary
      ? {
          total: summary.total,
          passed: summary.passed,
          failed: summary.total - summary.passed,
          errored: summary.errors,
          pass_rate: summary.pass_rate,
          cost_usd: summary.cost_per_case * summary.total,
          baseline_run_id: summary.baseline_run_id,
          baseline_reason: summary.baseline_run_id
            ? null
            : `No baseline on main for revision ${details.dataset_revision}`,
          pass_rate_delta: baseline
            ? summary.pass_rate - baseline.pass_rate
            : null,
          regressions: summary.regressions.map((item) => diff(details, item)),
          fixed: summary.fixed.map((item) => diff(details, item)),
        }
      : null,
  };
}
