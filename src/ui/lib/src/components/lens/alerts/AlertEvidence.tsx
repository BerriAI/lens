import { ArrowRight } from "lucide-react";
import { Button } from "../../ui/button";
import { Skeleton } from "../../ui/skeleton";
import { useTraceSignalState } from "../traces/list/useTraceSignals";
import type { OpenTraceRouting } from "../traces/routing";
import type { AlertActivity } from "./alertActivity";

export function AlertEvidence({
  accessToken,
  selected,
  onOpenSignal,
}: {
  accessToken: string;
  selected: AlertActivity["hits"][number];
  onOpenSignal: OpenTraceRouting["openSignal"];
}) {
  const result = useTraceSignalState(accessToken, selected.trace, true);
  const current =
    result?.status === "ready"
      ? result.signals.flags?.find((flag) => flag.signal_id === selected.flag.signal_id)
      : undefined;
  const flag = current ?? selected.flag;
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-auto p-6">
      <div className="mb-5 flex flex-wrap items-center justify-between gap-2 text-xs text-muted-foreground">
        <span>
          {new Intl.NumberFormat(undefined, { style: "percent", maximumFractionDigits: 1 }).format(
            flag.score,
          )}{" "}
          score
        </span>
        {selected.trace.classified_at && (
          <time dateTime={selected.trace.classified_at}>
            {new Date(selected.trace.classified_at).toLocaleString()}
          </time>
        )}
      </div>
      {result?.status === "pending" ? (
        <Skeleton aria-label="Loading alert evidence" className="h-24 w-full" />
      ) : current?.evidence?.quote ? (
        <blockquote className="border-l-2 border-primary bg-muted/30 px-4 py-3 text-sm leading-7 whitespace-pre-wrap wrap-anywhere">
          {current.evidence.quote}
        </blockquote>
      ) : (
        <p role="status" className="text-sm text-muted-foreground">
          {result?.status === "error"
            ? "Could not load the excerpt. You can still open the trace."
            : "No excerpt is available. Open the trace to inspect the conversation."}
        </p>
      )}
      <dl className="mt-6 space-y-2">
        <dt className="text-xs text-muted-foreground">Trace ID</dt>
        <dd className="font-mono text-xs break-all">{selected.trace.trace_id}</dd>
      </dl>
      <div className="mt-6 border-t pt-5">
        <Button
          onClick={() =>
            onOpenSignal({ traceId: selected.trace.trace_id, traceRef: selected.trace.trace_ref }, flag)
          }
        >
          Open trace
          <ArrowRight aria-hidden="true" />
        </Button>
      </div>
    </div>
  );
}
