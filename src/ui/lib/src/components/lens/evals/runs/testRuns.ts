import type {
  CaseDiff,
  EvalRun,
  RunCase,
  Summary,
  ToolStep,
  TrialSteps,
} from "./types";

export const diff = (
  case_id: string,
  overrides: Partial<CaseDiff> = {},
): CaseDiff => ({
  case_id,
  title: `Case ${case_id}`,
  critical: false,
  baseline_url: `https://lens.test/?eval_run=main&eval_case=${case_id}`,
  candidate_url: `https://lens.test/?eval_run=pr&eval_case=${case_id}`,
  ...overrides,
});

export const summary = (overrides: Partial<Summary> = {}): Summary => ({
  passed: 3,
  total: 4,
  pass_rate: 0.75,
  cost_per_case: 0.05,
  scores: {},
  errors: 0,
  baseline_run_id: "run-main",
  baseline_version: "1111111aaaa",
  regressions: [],
  fixed: [],
  gate: { passed: true, reasons: [] },
  ...overrides,
});

export const evalRun = (
  id: string,
  overrides: Partial<EvalRun> = {},
): EvalRun => ({
  id,
  status: "done",
  eval: "agent-regressions",
  agent: "refund-agent",
  version: "abcdef1234",
  branch: "main",
  pr: null,
  url: `https://lens.test/?eval_run=${id}`,
  expected_trials: 4,
  received_trials: 4,
  summary: summary(),
  failure: "",
  ...overrides,
});

export const step = (
  tool_name: string,
  start_ns: number,
  overrides: Partial<ToolStep> = {},
): ToolStep => ({
  name: `tool ${tool_name}`,
  tool_name,
  ok: true,
  start_ns,
  end_ns: start_ns + 5_000_000,
  ...overrides,
});

export const trial = (
  number: number,
  steps: readonly ToolStep[],
  overrides: Partial<TrialSteps> = {},
): TrialSteps => ({
  trial: number,
  error: null,
  checks: [],
  steps,
  ...overrides,
});

export const runCase = (
  passed: boolean | null,
  trials: readonly TrialSteps[],
): RunCase => ({
  case_id: "c1",
  title: "Fix the flaky test",
  critical: false,
  passed,
  trials,
});
