import { compareSteps, firstTrial } from "./steps";
import type { RunCase } from "./types";

export interface Diagnosis {
  readonly headline: string;
  readonly details: readonly string[];
}

const plural = (count: number, word: string) =>
  `${count} ${word}${count === 1 ? "" : "s"}`;

function failedChecks(runCase: RunCase): string[] {
  const failures = new Map<string, number>();
  for (const trial of runCase.trials)
    for (const check of trial.checks)
      if (!check.passed)
        failures.set(check.scorer, (failures.get(check.scorer) ?? 0) + 1);
  return [...failures].map(
    ([scorer, count]) =>
      `${scorer} failed ${count}/${runCase.trials.length} trials`,
  );
}

function stepChanges(candidate: RunCase, baseline: RunCase | null): string[] {
  const candidateSteps = firstTrial(candidate.trials)?.steps ?? [];
  const baselineSteps = baseline
    ? firstTrial(baseline.trials)?.steps ?? []
    : [];
  if (!baseline) return [];
  const compared = compareSteps(baselineSteps, candidateSteps);
  const skipped = compared.baseline
    .filter((step) => step.change === "skipped")
    .map((step) => step.step.tool_name);
  const added = compared.candidate
    .filter((step) => step.change === "added")
    .map((step) => step.step.tool_name);
  return [
    ...(skipped.length
      ? [`never called ${[...new Set(skipped)].join(", ")}, which main called`]
      : []),
    ...(added.length
      ? [`called ${[...new Set(added)].join(", ")}, which main did not`]
      : []),
  ];
}

export function diagnose(
  candidate: RunCase,
  baseline: RunCase | null,
): Diagnosis {
  const errors = candidate.trials.filter((trial) => trial.error !== null);
  const failed = failedChecks(candidate);
  const changes = stepChanges(candidate, baseline);
  const failedTools = candidate.trials.flatMap((trial) =>
    trial.steps.filter((step) => !step.ok).map((s) => s.tool_name),
  );
  const details = [
    ...failed,
    ...changes,
    ...(failedTools.length
      ? [`tool errors: ${[...new Set(failedTools)].join(", ")}`]
      : []),
    ...(errors.length
      ? [`${plural(errors.length, "trial")} errored: ${errors[0]?.error}`]
      : []),
  ];
  const headline =
    candidate.passed === false
      ? [changes[0], failed[0]].filter(Boolean).join(" · ") ||
        details[0] ||
        "Failed the judge scorer"
      : candidate.passed
        ? "Passes now" + (changes.length ? ` · ${changes[0]}` : "")
        : "Still scoring";
  return { headline, details };
}
