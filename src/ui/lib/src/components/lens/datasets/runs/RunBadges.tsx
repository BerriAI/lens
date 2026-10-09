import { cn } from "../../../../lib/cva.config";

import { gateTone, type GateTone } from "./format";
import type { EvalRun } from "./types";

const GATE: Record<GateTone, { readonly label: string; readonly className: string }> = {
  passed: { label: "Passed", className: "bg-success/10 text-success" },
  failed: { label: "Failed", className: "bg-destructive/10 text-destructive" },
  pending: { label: "Scoring", className: "bg-muted text-muted-foreground" },
  errored: { label: "Errored", className: "bg-warning/12 text-warning" },
};

export const PILL = "inline-flex h-5 shrink-0 items-center gap-1.5 rounded-full px-2 text-xs font-medium";

export function GatePill({ run }: { run: EvalRun }) {
  const { label, className } = GATE[gateTone(run)];
  return (
    <span className={cn(PILL, className)}>
      <span aria-hidden="true" className="size-1.5 rounded-full bg-current" />
      Gate {label.toLowerCase()}
    </span>
  );
}

export function CriticalPill() {
  return <span className={cn(PILL, "bg-destructive/10 text-destructive")}>Critical</span>;
}

export function PullRequestPill({ pr }: { pr: number | null }) {
  if (pr === null) return null;
  return <span className={cn(PILL, "bg-muted font-mono text-muted-foreground")}>PR #{pr}</span>;
}
