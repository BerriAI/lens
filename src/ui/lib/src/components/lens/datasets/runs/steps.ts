import type { Span } from "../../traces/types";
import { isFrameworkSpan } from "../../traces/utils";

export type StepChange = "same" | "skipped" | "added";

export interface ToolStep {
  readonly span: Span;
  readonly change: StepChange;
}

export interface StepComparison {
  readonly baseline: readonly ToolStep[];
  readonly candidate: readonly ToolStep[];
}

export const toolSteps = (spans: readonly Span[]): Span[] =>
  spans
    .filter((span) => span.type === "tool" && !isFrameworkSpan(span))
    .sort((a, b) => a.start_offset_ms - b.start_offset_ms);

const commonSuffixLengths = (a: readonly string[], b: readonly string[]): number[][] => {
  const table = Array.from({ length: a.length + 1 }, () => new Array<number>(b.length + 1).fill(0));
  for (let i = a.length - 1; i >= 0; i--)
    for (let j = b.length - 1; j >= 0; j--)
      table[i][j] = a[i] === b[j] ? table[i + 1][j + 1] + 1 : Math.max(table[i + 1][j], table[i][j + 1]);
  return table;
};

interface Matches {
  readonly baseline: ReadonlySet<number>;
  readonly candidate: ReadonlySet<number>;
}

function longestCommonSteps(a: readonly string[], b: readonly string[]): Matches {
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

export function compareSteps(baseline: readonly Span[], candidate: readonly Span[]): StepComparison {
  const matches = longestCommonSteps(
    baseline.map((span) => span.name),
    candidate.map((span) => span.name),
  );
  return {
    baseline: baseline.map((span, index) => ({ span, change: matches.baseline.has(index) ? "same" : "skipped" })),
    candidate: candidate.map((span, index) => ({ span, change: matches.candidate.has(index) ? "same" : "added" })),
  };
}

export const unchangedSteps = (spans: readonly Span[]): ToolStep[] => spans.map((span) => ({ span, change: "same" }));
