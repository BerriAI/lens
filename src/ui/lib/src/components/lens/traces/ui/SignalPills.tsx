import { cn } from "../../../../lib/cva.config";

import type { SignalFlag } from "../types";

const percent = (score: number): string => `${Math.round(score * 100)}%`;

export const signalSummary = (flags: readonly SignalFlag[]): string =>
  flags.map((flag) => `${flag.name} (${percent(flag.score)})`).join(", ");

function SignalPill({ flag, showScore, onSelect }: { flag: SignalFlag; showScore: boolean; onSelect?: () => void }) {
  const content = (
    <>
      <span aria-hidden="true" className="size-1.5 shrink-0 rounded-full bg-destructive" />
      <span className="truncate">{flag.name}</span>
      {showScore && <span className="shrink-0 font-normal tabular-nums opacity-80">{percent(flag.score)}</span>}
    </>
  );
  const className =
    "inline-flex max-w-full min-w-0 items-center gap-1 rounded-full bg-destructive/10 px-1.5 py-0.5 text-xs leading-none font-medium text-destructive";
  if (onSelect)
    return (
      <button
        type="button"
        aria-label={`View ${flag.name} evidence`}
        onClick={(event) => {
          event.stopPropagation();
          onSelect();
        }}
        className={cn(
          className,
          "cursor-pointer hover:bg-destructive/20 focus-visible:outline-2 focus-visible:outline-ring",
        )}
      >
        {content}
      </button>
    );
  return <span className={className}>{content}</span>;
}

export function SignalPills({
  flags,
  showScore = false,
  className,
  onSelect,
}: {
  flags: readonly SignalFlag[];
  showScore?: boolean;
  className?: string;
  onSelect?: (flag: SignalFlag) => void;
}) {
  return (
    <span
      role="list"
      aria-label="Signals"
      title={`Signals: ${signalSummary(flags)}`}
      className={cn("inline-flex max-w-full min-w-0 items-center gap-1", className)}
    >
      {flags.map((flag) => (
        <span role="listitem" key={flag.signal_id} className="min-w-0">
          <SignalPill flag={flag} showScore={showScore} onSelect={onSelect ? () => onSelect(flag) : undefined} />
        </span>
      ))}
    </span>
  );
}
