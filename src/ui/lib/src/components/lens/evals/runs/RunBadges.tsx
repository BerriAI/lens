import { CheckCircle2, Circle, Loader2, OctagonAlert, XCircle } from "lucide-react";

import { cn } from "../../../../lib/cva.config";
import type { EvalRun, RunCaseSummary } from "./types";

export type CaseStatus = "Passed" | "Failed" | "Pending" | "Not scored" | "Error";

export function caseStatus(item: Pick<RunCaseSummary, "passed">, active: boolean): CaseStatus {
  if (item.passed === true) return "Passed";
  if (item.passed === false) return "Failed";
  return active ? "Pending" : "Not scored";
}

const BADGES = {
  Passed: { icon: CheckCircle2, tone: "text-success" },
  Failed: { icon: XCircle, tone: "text-destructive" },
  Error: { icon: OctagonAlert, tone: "text-destructive" },
  Running: { icon: Loader2, tone: "text-blue-600 dark:text-blue-400" },
  Scoring: { icon: Loader2, tone: "text-blue-600 dark:text-blue-400" },
  Pending: { icon: Circle, tone: "text-muted-foreground" },
  "Not scored": { icon: Circle, tone: "text-muted-foreground" },
  "No results": { icon: Circle, tone: "text-muted-foreground" },
} as const;

export function StatusBadge({ status, className }: { status: keyof typeof BADGES; className?: string }) {
  const { icon: Icon, tone } = BADGES[status];
  return (
    <span className={cn("inline-flex items-center gap-1.5 whitespace-nowrap text-xs font-medium", tone, className)}>
      <Icon
        aria-hidden="true"
        className={cn(
          "size-3.5 shrink-0",
          (status === "Running" || status === "Scoring") && "animate-spin motion-reduce:animate-none",
        )}
      />
      {status}
    </span>
  );
}

export function RunStatusBadge({ run, className }: { run: EvalRun; className?: string }) {
  const status =
    run.status === "failed"
      ? "Error"
      : run.status === "running"
        ? "Running"
        : run.status === "scoring"
          ? "Scoring"
          : !run.summary || run.summary.total === 0
            ? "No results"
            : run.summary.passed === run.summary.total
              ? "Passed"
              : "Failed";
  return <StatusBadge status={status} className={className} />;
}

export const GateStatus = RunStatusBadge;

export function CriticalTag() {
  return <span className="rounded border px-1 text-[10px] leading-4 text-muted-foreground">Critical</span>;
}
