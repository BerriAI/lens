import { cn } from "../../../../lib/cva.config";

import { gateTone, type GateTone } from "./format";
import type { EvalRun } from "./types";

const GATE: Record<
  GateTone,
  { readonly label: string; readonly className: string }
> = {
  passed: { label: "passed", className: "text-success" },
  failed: { label: "failed", className: "text-destructive" },
  pending: { label: "scoring", className: "text-muted-foreground" },
  errored: { label: "errored", className: "text-warning" },
};

export function GateStatus({
  run,
  className,
}: {
  run: EvalRun;
  className?: string;
}) {
  const { label, className: tone } = GATE[gateTone(run)];
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 font-mono text-xs whitespace-nowrap",
        tone,
        className,
      )}
    >
      <span aria-hidden="true" className="size-2 rounded-[2px] bg-current" />
      gate {label}
    </span>
  );
}

export function CriticalTag() {
  return (
    <span className="rounded-[3px] border border-destructive/40 px-1 font-mono text-[10px] leading-4 text-destructive uppercase">
      crit
    </span>
  );
}
