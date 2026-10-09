"use client";

import { CircleAlert, CircleCheck, Loader2, X } from "lucide-react";
import { useMemo } from "react";

import { Button } from "../../../ui/button";
import { cn } from "../../../../lib/cva.config";

import { useRunCase } from "./api";
import { CriticalPill, PILL } from "./RunBadges";
import { compareSteps, firstTrial, unchangedSteps, type ComparedStep, type StepChange } from "./steps";
import type { CaseDiff, RunCase } from "./types";

export interface CaseCompareProps {
  readonly diff: CaseDiff;
  readonly runId: string;
  readonly baselineRunId: string | null;
  readonly regressed: boolean;
  readonly onClose: () => void;
}

export function CaseCompare({ diff, runId, baselineRunId, regressed, onClose }: CaseCompareProps) {
  const baseline = useRunCase(baselineRunId, diff.case_id);
  const candidate = useRunCase(runId, diff.case_id);
  const compared = useMemo(() => {
    const baselineSteps = firstTrial(baseline.data?.trials ?? [])?.steps ?? [];
    const candidateSteps = firstTrial(candidate.data?.trials ?? [])?.steps ?? [];
    return baseline.data && candidate.data
      ? compareSteps(baselineSteps, candidateSteps)
      : { baseline: unchangedSteps(baselineSteps), candidate: unchangedSteps(candidateSteps) };
  }, [baseline.data, candidate.data]);
  return (
    <section aria-label="Case comparison" className="flex flex-col gap-3">
      <header className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <h4 className="flex items-center gap-2 text-sm font-medium text-foreground">
            {regressed ? "Regressed case" : "Fixed case"}
            {diff.critical && <CriticalPill />}
          </h4>
          <p className="mt-0.5 line-clamp-2 text-sm text-muted-foreground">{diff.title || diff.case_id}</p>
        </div>
        <Button size="sm" variant="ghost" onClick={onClose} aria-label="Back to run">
          <X className="size-3.5" />
        </Button>
      </header>
      <div className="grid gap-3 md:grid-cols-2">
        <Trajectory
          title="Baseline (main)"
          runCase={baselineRunId === null ? null : baseline.data ?? null}
          missing={baselineRunId === null}
          steps={compared.baseline}
          loading={baseline.isLoading}
          error={baseline.error}
        />
        <Trajectory
          title="Candidate"
          runCase={candidate.data ?? null}
          missing={false}
          steps={compared.candidate}
          loading={candidate.isLoading}
          error={candidate.error}
        />
      </div>
    </section>
  );
}

interface TrajectoryProps {
  readonly title: string;
  readonly runCase: RunCase | null;
  readonly missing: boolean;
  readonly steps: readonly ComparedStep[];
  readonly loading: boolean;
  readonly error: Error | null;
}

function Verdict({ runCase }: { runCase: RunCase | null }) {
  if (runCase?.passed == null) return null;
  return runCase.passed ? (
    <span className={cn(PILL, "bg-success/10 text-success")}>Pass</span>
  ) : (
    <span className={cn(PILL, "bg-destructive/10 text-destructive")}>Fail</span>
  );
}

function Trajectory(props: TrajectoryProps) {
  const trials = props.runCase?.trials.length ?? 0;
  return (
    <section aria-label={`${props.title} trajectory`} className="flex min-w-0 flex-col rounded-xl border bg-card">
      <header className="flex min-h-11 items-center gap-2 px-4 pt-3">
        <h5 className="text-sm font-medium text-foreground">{props.title}</h5>
        <Verdict runCase={props.runCase} />
        {trials > 1 && <span className="ml-auto text-xs text-muted-foreground">trial 1 of {trials}</span>}
      </header>
      <div className="px-4 pt-2 pb-4">
        <TrajectoryBody {...props} />
      </div>
    </section>
  );
}

function TrajectoryBody({ title, runCase, missing, steps, loading, error }: TrajectoryProps) {
  if (missing) return <p className="text-sm text-muted-foreground">No baseline run for this case.</p>;
  if (loading)
    return (
      <p role="status" className="flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="size-3.5 animate-spin motion-reduce:animate-none" />
        Loading trace…
      </p>
    );
  if (error)
    return (
      <p role="alert" className="text-sm text-destructive">
        {error.message}
      </p>
    );
  const trialError = runCase ? firstTrial(runCase.trials)?.error : null;
  if (trialError)
    return (
      <p role="alert" className="text-sm text-destructive">
        {trialError}
      </p>
    );
  if (steps.length === 0) return <p className="text-sm text-muted-foreground">No tool calls recorded.</p>;
  return (
    <ol aria-label={`${title} tool steps`} className="flex flex-col gap-1">
      {steps.map((step, index) => (
        <Step key={`${index}-${step.step.tool_name}`} index={index} step={step} />
      ))}
    </ol>
  );
}

const CHANGE: Record<StepChange, { readonly label: string; readonly row: string; readonly badge: string }> = {
  same: { label: "", row: "", badge: "" },
  skipped: {
    label: "Skipped in candidate",
    row: "bg-destructive/5 ring-1 ring-destructive/30",
    badge: "bg-destructive/10 text-destructive",
  },
  added: { label: "Added", row: "bg-warning/5 ring-1 ring-warning/30", badge: "bg-warning/12 text-warning" },
};

function Step({ step, index }: { step: ComparedStep; index: number }) {
  const style = CHANGE[step.change];
  const Icon = step.step.ok ? CircleCheck : CircleAlert;
  return (
    <li className={cn("flex min-w-0 items-center gap-2 rounded-md px-2 py-1.5", style.row)}>
      <span className="w-4 shrink-0 text-right text-xs text-muted-foreground tabular-nums">{index + 1}</span>
      <Icon
        aria-hidden="true"
        className={cn("size-3.5 shrink-0", step.step.ok ? "text-muted-foreground" : "text-destructive")}
      />
      <span className="min-w-0 flex-1 truncate font-mono text-xs text-foreground">{step.step.tool_name}</span>
      {style.label && <span className={cn(PILL, style.badge)}>{style.label}</span>}
    </li>
  );
}
