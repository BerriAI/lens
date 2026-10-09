"use client";

import { Activity, ArrowRight, RefreshCw } from "lucide-react";
import { Button } from "../../ui/button";

export function WaitingForTraces({
  onConnect,
  onCheck,
  checking,
  detail,
}: {
  readonly onConnect?: () => void;
  readonly onCheck: () => void;
  readonly checking: boolean;
  readonly detail?: string | null;
}) {
  return (
    <section
      aria-label="Waiting for traces"
      className="my-5 flex min-h-96 flex-1 flex-col items-center rounded-xl border bg-muted/20 px-5 py-12 sm:py-20"
    >
      <div className="w-full max-w-xl">
        <span
          role="status"
          className="inline-flex items-center gap-2 rounded-full bg-primary px-3 py-1.5 text-xs font-medium text-primary-foreground"
        >
          <Activity aria-hidden className="size-3.5" />
          Waiting for traces
        </span>
        <h2 className="mt-5 text-xl font-semibold tracking-tight">Connect your project to see its traces</h2>
        <p className="mt-2 text-sm leading-6 text-muted-foreground">
          Home walks you through setup and checks the connection live. Your agent appears here automatically when its
          traces arrive.
        </p>
        {onConnect && (
          <Button className="mt-6" onClick={onConnect}>
            Connect project
            <ArrowRight aria-hidden className="size-4" />
          </Button>
        )}
        {detail && (
          <p role="alert" className="mt-4 text-sm text-muted-foreground">
            Trace storage is not configured yet. Check the live connection panel on Home.
          </p>
        )}
        <div className="mt-4">
          <Button
            variant="ghost"
            size="sm"
            disabled={checking}
            onClick={onCheck}
            className="px-0 text-xs text-muted-foreground"
          >
            <RefreshCw aria-hidden className="size-3.5" />
            {checking ? "Checking…" : "Check for traces"}
          </Button>
        </div>
      </div>
    </section>
  );
}
