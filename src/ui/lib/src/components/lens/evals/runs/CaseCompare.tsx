"use client";

import { useMemo, useState } from "react";

import { cn } from "../../../../lib/cva.config";

import { useRunCase } from "./api";
import { diagnose } from "./diagnosis";
import { shortSha } from "./format";
import { CriticalTag } from "./RunBadges";
import { compareSteps, unchangedSteps, type ComparedStep } from "./steps";
import type { CaseDiff, RunCase, TrialSteps } from "./types";

export interface CaseCompareProps {
  readonly diff: CaseDiff;
  readonly runId: string;
  readonly baselineRunId: string | null;
  readonly baselineVersion: string | null;
  readonly candidateVersion: string;
  readonly regressed: boolean;
}

export function CaseCompare({
  diff,
  runId,
  baselineRunId,
  baselineVersion,
  candidateVersion,
  regressed,
}: CaseCompareProps) {
  const baseline = useRunCase(baselineRunId, diff.case_id);
  const candidate = useRunCase(runId, diff.case_id);
  const [trialIndex, setTrialIndex] = useState(0);
  const baselineTrial =
    baseline.data?.trials[
      Math.min(trialIndex, baseline.data.trials.length - 1)
    ] ?? null;
  const candidateTrial = candidate.data?.trials[trialIndex] ?? null;
  const compared = useMemo(() => {
    const before = baselineTrial?.steps ?? [];
    const after = candidateTrial?.steps ?? [];
    return baseline.data && candidate.data
      ? compareSteps(before, after)
      : { baseline: unchangedSteps(before), candidate: unchangedSteps(after) };
  }, [baseline.data, candidate.data, baselineTrial, candidateTrial]);
  const diagnosis = candidate.data
    ? diagnose(candidate.data, baseline.data ?? null)
    : null;
  return (
    <section aria-label="Case comparison" className="flex flex-col">
      <header className="border-b px-3 py-2">
        <div className="flex items-center gap-2">
          <h4 className="min-w-0 flex-1 truncate text-sm font-medium text-foreground">
            {diff.title || diff.case_id}
          </h4>
          {diff.critical && <CriticalTag />}
          <span className="font-mono text-[11px] text-muted-foreground">
            {diff.case_id.slice(0, 12)}
          </span>
        </div>
        <p
          aria-label="Diagnosis"
          className={cn(
            "mt-1 font-mono text-xs",
            regressed ? "text-destructive" : "text-success",
          )}
        >
          {regressed ? "✕ regressed: " : "✓ fixed: "}
          {diagnosis
            ? diagnosis.headline
            : candidate.error
              ? candidate.error.message
              : "reading traces…"}
        </p>
        {diagnosis && diagnosis.details.length > 1 && (
          <ul className="mt-0.5 font-mono text-[11px] text-muted-foreground">
            {diagnosis.details.map((detail) => (
              <li key={detail}>· {detail}</li>
            ))}
          </ul>
        )}
      </header>
      {candidate.data && candidate.data.trials.length > 1 && (
        <TrialTabs
          trials={candidate.data.trials}
          selected={trialIndex}
          onSelect={setTrialIndex}
        />
      )}
      <div className="grid min-w-0 md:grid-cols-2 md:divide-x">
        <Trace
          title="main"
          version={baselineVersion}
          runCase={baselineRunId === null ? null : baseline.data ?? null}
          trial={baselineTrial}
          steps={compared.baseline}
          state={
            baselineRunId === null
              ? "No baseline run"
              : baseline.error?.message ??
                (baseline.isLoading ? "loading" : null)
          }
        />
        <Trace
          title="candidate"
          version={candidateVersion}
          runCase={candidate.data ?? null}
          trial={candidateTrial}
          steps={compared.candidate}
          state={
            candidate.error?.message ?? (candidate.isLoading ? "loading" : null)
          }
        />
      </div>
    </section>
  );
}

function TrialTabs({
  trials,
  selected,
  onSelect,
}: {
  trials: readonly TrialSteps[];
  selected: number;
  onSelect: (index: number) => void;
}) {
  return (
    <div
      role="tablist"
      aria-label="Trials"
      className="flex border-b font-mono text-xs"
    >
      {trials.map((trial, index) => {
        const failed =
          trial.error !== null || trial.checks.some((check) => !check.passed);
        return (
          <button
            key={trial.trial}
            type="button"
            role="tab"
            aria-selected={index === selected}
            onClick={() => onSelect(index)}
            className={cn(
              "flex items-center gap-1.5 border-r border-b-2 border-b-transparent px-3 py-1.5 text-muted-foreground hover:text-foreground",
              index === selected && "border-b-foreground text-foreground",
            )}
          >
            <span
              aria-hidden="true"
              className={cn(
                "size-1.5 rounded-full",
                failed ? "bg-destructive" : "bg-success",
              )}
            />
            trial {trial.trial}
          </button>
        );
      })}
    </div>
  );
}

interface TraceProps {
  readonly title: string;
  readonly version: string | null;
  readonly runCase: RunCase | null;
  readonly trial: TrialSteps | null;
  readonly steps: readonly ComparedStep[];
  readonly state: string | null;
}

function Trace({ title, version, runCase, trial, steps, state }: TraceProps) {
  return (
    <section aria-label={`${title} trace`} className="min-w-0">
      <h5 className="flex items-center gap-2 border-b bg-muted/30 px-3 py-1 font-mono text-[11px]">
        <span className="text-foreground uppercase">{title}</span>
        {version && (
          <span className="text-muted-foreground">@{shortSha(version)}</span>
        )}
        <span className="ml-auto">
          {runCase?.passed === true && (
            <span className="text-success">pass</span>
          )}
          {runCase?.passed === false && (
            <span className="text-destructive">fail</span>
          )}
        </span>
      </h5>
      {trial && trial.checks.length > 0 && (
        <ul
          aria-label={`${title} checks`}
          className="flex flex-wrap gap-x-3 border-b px-3 py-1 font-mono text-[11px]"
        >
          {trial.checks.map((check) => (
            <li
              key={check.scorer}
              className={
                check.passed ? "text-muted-foreground" : "text-destructive"
              }
            >
              {check.passed ? "✓" : "✕"} {check.scorer}
            </li>
          ))}
        </ul>
      )}
      {trial?.error && (
        <p className="border-b px-3 py-1 font-mono text-[11px] text-warning">
          error: {trial.error}
        </p>
      )}
      {state ? (
        <p className="px-3 py-3 font-mono text-xs text-muted-foreground">
          {state}
        </p>
      ) : (
        <Waterfall steps={steps} />
      )}
    </section>
  );
}

const CHANGE_ROW: Record<ComparedStep["change"], string> = {
  same: "",
  skipped: "bg-destructive/8",
  added: "bg-warning/8",
};

function Waterfall({ steps }: { steps: readonly ComparedStep[] }) {
  if (steps.length === 0)
    return (
      <p className="px-3 py-3 font-mono text-xs text-muted-foreground">
        No tool calls
      </p>
    );
  const start = Math.min(...steps.map(({ step }) => step.start_ns));
  const end = Math.max(
    ...steps.map(({ step }) => Math.max(step.end_ns, step.start_ns)),
  );
  const span = Math.max(end - start, 1);
  return (
    <ol aria-label="Tool calls" className="font-mono text-xs">
      {steps.map(({ step, change }, index) => {
        const duration = Math.max(step.end_ns - step.start_ns, 0);
        return (
          <li
            key={`${index}-${step.start_ns}`}
            aria-label={`${step.tool_name}${change === "skipped" ? " (missing in candidate)" : change === "added" ? " (new in candidate)" : ""}`}
            className={cn(
              "grid grid-cols-[1.75rem_9rem_1fr_3.5rem] items-center gap-2 border-b px-3 py-1",
              CHANGE_ROW[change],
            )}
          >
            <span className="text-right text-muted-foreground tabular-nums">
              {index + 1}
            </span>
            <span
              className={cn(
                "truncate",
                step.ok ? "text-foreground" : "text-destructive",
              )}
              title={step.name}
            >
              {step.tool_name}
              {change === "skipped" && (
                <span className="ml-1 text-destructive">−</span>
              )}
              {change === "added" && (
                <span className="ml-1 text-warning">+</span>
              )}
            </span>
            <span className="relative h-2.5 rounded-[2px] bg-muted/60">
              <span
                className={cn(
                  "absolute inset-y-0 min-w-[2px] rounded-[2px]",
                  !step.ok
                    ? "bg-destructive"
                    : change === "skipped"
                      ? "bg-destructive/60"
                      : change === "added"
                        ? "bg-warning"
                        : "bg-foreground/50",
                )}
                style={{
                  left: `${((step.start_ns - start) / span) * 100}%`,
                  width: `${(duration / span) * 100}%`,
                }}
              />
            </span>
            <span className="text-right text-muted-foreground tabular-nums">
              {formatDuration(duration)}
            </span>
          </li>
        );
      })}
    </ol>
  );
}

const formatDuration = (ns: number): string => {
  const ms = ns / 1e6;
  if (ms < 1000) return `${Math.round(ms)}ms`;
  return `${(ms / 1000).toFixed(ms < 10_000 ? 2 : 1)}s`;
};
