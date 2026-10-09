import type { ToolStep, TrialSteps } from "./types";

export type StepChange = "same" | "skipped" | "added";

export interface ComparedStep {
  readonly step: ToolStep;
  readonly change: StepChange;
}

export interface StepComparison {
  readonly baseline: readonly ComparedStep[];
  readonly candidate: readonly ComparedStep[];
}

export const firstTrial = (trials: readonly TrialSteps[]): TrialSteps | null =>
  trials[0] ?? null;

const commonSuffixLengths = (
  a: readonly string[],
  b: readonly string[],
): number[][] => {
  const table = Array.from({ length: a.length + 1 }, () =>
    new Array<number>(b.length + 1).fill(0),
  );
  for (let i = a.length - 1; i >= 0; i--)
    for (let j = b.length - 1; j >= 0; j--)
      table[i][j] =
        a[i] === b[j]
          ? table[i + 1][j + 1] + 1
          : Math.max(table[i + 1][j], table[i][j + 1]);
  return table;
};

interface Matches {
  readonly baseline: ReadonlySet<number>;
  readonly candidate: ReadonlySet<number>;
}

function longestCommonSteps(
  a: readonly string[],
  b: readonly string[],
): Matches {
  const table = commonSuffixLengths(a, b);
  const baseline = new Set<number>();
  const candidate = new Set<number>();
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      baseline.add(i++);
      candidate.add(j++);
    } else if (table[i + 1][j] >= table[i][j + 1]) i++;
    else j++;
  }
  return { baseline, candidate };
}

export function compareSteps(
  baseline: readonly ToolStep[],
  candidate: readonly ToolStep[],
): StepComparison {
  const matches = longestCommonSteps(
    baseline.map((step) => step.tool_name),
    candidate.map((step) => step.tool_name),
  );
  return {
    baseline: baseline.map((step, index) => ({
      step,
      change: matches.baseline.has(index) ? "same" : "skipped",
    })),
    candidate: candidate.map((step, index) => ({
      step,
      change: matches.candidate.has(index) ? "same" : "added",
    })),
  };
}

export const unchangedSteps = (steps: readonly ToolStep[]): ComparedStep[] =>
  steps.map((step) => ({ step, change: "same" }));
