import { SquareArrowOutUpRight } from "lucide-react";

import { cn } from "@/lib/cva.config";

import { gateTone, type GateTone } from "./format";
import type { EvalRun } from "./types";

const GATE: Record<GateTone, { readonly label: string; readonly className: string }> = {
  passed: { label: "Passed", className: "bg-success/10 text-success" },
  failed: { label: "Failed", className: "bg-destructive/10 text-destructive" },
  pending: { label: "Scoring", className: "bg-muted text-muted-foreground" },
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

export function PullRequestLink({ url }: { url: string }) {
  return (
    <a
      href={url}
      target="_blank"
      rel="noreferrer"
      className="inline-flex items-center gap-1 rounded text-xs text-muted-foreground hover:text-foreground hover:underline focus-visible:outline-2 focus-visible:outline-ring"
    >
      Pull request
      <SquareArrowOutUpRight aria-hidden="true" className="size-3.5" />
    </a>
  );
}
