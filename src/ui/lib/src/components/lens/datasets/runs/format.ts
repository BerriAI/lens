import type { EvalRun, Summary } from "./types";

export type GateTone = "passed" | "failed" | "pending" | "errored";

export const gateTone = (run: EvalRun): GateTone => {
  if (run.status === "error") return "errored";
  if (run.gate === null) return "pending";
  return run.gate.passed ? "passed" : "failed";
};

export const passedLabel = (summary: Summary): string => `${summary.passed}/${summary.total}`;

const percentPoints = (rate: number): string => `${Math.abs(rate * 100).toFixed(1)} pts`;

export const deltaLabel = (summary: Summary): string => {
  if (summary.pass_rate_delta === null) return "No baseline";
  if (summary.pass_rate_delta === 0) return "No change";
  return `${summary.pass_rate_delta > 0 ? "+" : "−"}${percentPoints(summary.pass_rate_delta)}`;
};

export const deltaTone = (delta: number | null): string => {
  if (delta === null || delta === 0) return "text-muted-foreground";
  return delta < 0 ? "text-destructive" : "text-success";
};

export const costPerCase = (summary: Summary): string => {
  if (summary.cost_usd === null || summary.total === 0) return "–";
  const each = summary.cost_usd / summary.total;
  return `$${each.toFixed(each < 0.01 ? 4 : 2)}`;
};

export const shortSha = (sha: string): string => sha.slice(0, 7);

export const safeLinkUrl = (url: string | null): string | null => (url && /^https?:\/\/[^/\s]/i.test(url) ? url : null);

export interface RunGroups {
  readonly main: readonly EvalRun[];
  readonly pulls: readonly EvalRun[];
}

const newestFirst = (a: EvalRun, b: EvalRun): number => b.created_at.localeCompare(a.created_at);

export const groupRuns = (runs: readonly EvalRun[], datasetId: string): RunGroups => {
  const ours = runs.filter((run) => run.dataset_id === datasetId).sort(newestFirst);
  return {
    main: ours.filter((run) => run.pr_url === null && run.pr_number == null),
    pulls: ours.filter((run) => run.pr_url !== null || run.pr_number != null),
  };
};
