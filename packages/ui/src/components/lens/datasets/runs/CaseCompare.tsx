"use client";

import { Loader2, X } from "lucide-react";
import { useMemo } from "react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/cva.config";

import { toolSummary } from "../../traces/detail/content/payload";
import { SpanIcon } from "../../traces/ui/SpanIcon";
import { fmtMs } from "../../traces/utils";
import { useCaseTrace } from "./api";
import { CriticalPill, PILL } from "./RunBadges";
import { compareSteps, toolSteps, unchangedSteps, type StepChange, type ToolStep } from "./steps";
import type { CaseDiff, CaseOutcome, Verdict } from "./types";

export interface CaseCompareProps {
  readonly diff: CaseDiff;
  readonly onClose: () => void;
}

export function CaseCompare({ diff, onClose }: CaseCompareProps) {
  const baseline = useCaseTrace(diff.baseline);
  const candidate = useCaseTrace(diff.candidate);
  const compared = useMemo(() => {
    const baselineSpans = toolSteps(baseline.data?.spans ?? []);
    const candidateSpans = toolSteps(candidate.data?.spans ?? []);
    return baseline.data && candidate.data
      ? compareSteps(baselineSpans, candidateSpans)
      : { baseline: unchangedSteps(baselineSpans), candidate: unchangedSteps(candidateSpans) };
  }, [baseline.data, candidate.data]);
  return (
    <section aria-label="Case comparison" className="flex flex-col gap-3">
      <header className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <h4 className="flex items-center gap-2 text-sm font-medium text-foreground">
            Case
            {diff.critical && <CriticalPill />}
          </h4>
          <p className="mt-0.5 line-clamp-2 text-sm text-muted-foreground">{diff.input || diff.case_id}</p>
        </div>
        <Button size="sm" variant="ghost" onClick={onClose} aria-label="Back to run">
          <X className="size-3.5" />
        </Button>
      </header>
      <div className="grid gap-3 md:grid-cols-2">
        <Trajectory
          title="Baseline"
          outcome={diff.baseline}
          steps={compared.baseline}
          loading={baseline.isLoading}
          error={baseline.error}
        />
        <Trajectory
          title="Candidate"
          outcome={diff.candidate}
          steps={compared.candidate}
          loading={candidate.isLoading}
          error={candidate.error}
        />
      </div>
    </section>
  );
}

const VERDICT: Record<Verdict, { readonly label: string; readonly className: string }> = {
  pass: { label: "Pass", className: "bg-success/10 text-success" },
  fail: { label: "Fail", className: "bg-destructive/10 text-destructive" },
  error: { label: "Error", className: "bg-warning/12 text-warning" },
};

interface TrajectoryProps {
  readonly title: string;
  readonly outcome: CaseOutcome | null;
  readonly steps: readonly ToolStep[];
  readonly loading: boolean;
  readonly error: Error | null;
}

function Trajectory({ title, outcome, steps, loading, error }: TrajectoryProps) {
  const verdict = outcome ? VERDICT[outcome.verdict] : null;
  return (
    <section aria-label={`${title} trajectory`} className="flex min-w-0 flex-col rounded-xl border bg-card">
      <header className="flex min-h-11 items-center gap-2 px-4 pt-3">
        <h5 className="text-sm font-medium text-foreground">{title}</h5>
        {verdict && <span className={cn(PILL, verdict.className)}>{verdict.label}</span>}
      </header>
      <div className="px-4 pt-2 pb-4">
        <TrajectoryBody title={title} outcome={outcome} steps={steps} loading={loading} error={error} />
      </div>
    </section>
  );
}

function TrajectoryBody({ title, outcome, steps, loading, error }: TrajectoryProps) {
  if (outcome === null) return <p className="text-sm text-muted-foreground">No baseline run for this case.</p>;
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
  if (steps.length === 0) return <p className="text-sm text-muted-foreground">No tool calls recorded.</p>;
  return (
    <ol aria-label={`${title} tool steps`} className="flex flex-col gap-1">
      {steps.map((step) => (
        <Step key={step.span.span_id} step={step} />
      ))}
    </ol>
  );
}

const CHANGE: Record<StepChange, { readonly label: string; readonly row: string; readonly badge: string }> = {
  same: { label: "", row: "", badge: "" },
  skipped: {
    label: "Skipped",
    row: "bg-destructive/5 ring-1 ring-destructive/30",
    badge: "bg-destructive/10 text-destructive",
  },
  added: { label: "Added", row: "bg-warning/5 ring-1 ring-warning/30", badge: "bg-warning/12 text-warning" },
};

function Step({ step }: { step: ToolStep }) {
  const { span, change } = step;
  const style = CHANGE[change];
  const failed = span.status === "error";
  const summary = toolSummary(span.input_preview);
  return (
    <li className={cn("flex min-w-0 items-start gap-2 rounded-md px-2 py-1.5", style.row)}>
      <SpanIcon type={span.type} error={failed} size="sm" />
      <div className="flex min-w-0 flex-1 flex-col">
        <span className={cn("truncate font-mono text-xs", failed ? "text-destructive" : "text-foreground")}>
          {span.name}
        </span>
        {summary && <span className="truncate text-xs text-muted-foreground">{summary}</span>}
      </div>
      {style.label && <span className={cn(PILL, style.badge)}>{style.label}</span>}
      <span className="shrink-0 text-xs text-muted-foreground tabular-nums">{fmtMs(span.duration_ms)}</span>
    </li>
  );
}
