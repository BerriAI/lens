"use client";

import { ArrowLeft, Loader2 } from "lucide-react";
import { useState } from "react";

import { cn } from "../../../../lib/cva.config";
import { Button } from "../../../ui/button";
import { useLensAccessToken } from "../../data/LensServices";
import { RunView } from "../../traces/detail/run/RunView";
import { useLocalRunSelection } from "../../traces/routing";
import { useRunCase } from "./api";
import { caseStatus, StatusBadge } from "./RunBadges";
import type { RunCaseSummary, RunStatus, TrialSteps, TrialTrace } from "./types";

export interface CaseCompareProps {
  readonly runId: string;
  readonly runStatus: RunStatus;
  readonly item: RunCaseSummary;
  readonly onBack: () => void;
}

export function CaseCompare({ runId, runStatus, item, onBack }: CaseCompareProps) {
  const active = runStatus === "running" || runStatus === "scoring";
  const result = useRunCase(runId, item.case_id, active);
  const [trialIndex, setTrialIndex] = useState(0);
  const trial = result.data?.trials[trialIndex];
  const status = caseStatus(result.data ?? item, active);
  return (
    <section aria-label="Case result" className="flex min-h-0 flex-1 flex-col">
      <header className="space-y-2 border-b px-5 py-4">
        <Button variant="ghost" size="sm" className="-ml-2 h-7 text-muted-foreground" onClick={onBack}>
          <ArrowLeft className="size-3.5" />
          All cases
        </Button>
        <div className="flex flex-wrap items-center gap-3">
          <h3 className="min-w-0 flex-1 text-sm font-semibold">{item.title || item.case_id}</h3>
          <StatusBadge status={status} />
        </div>
      </header>
      {result.isPending ? (
        <p role="status" className="flex items-center gap-2 p-5 text-sm text-muted-foreground">
          <Loader2 className="size-4 animate-spin motion-reduce:animate-none" />
          Loading case…
        </p>
      ) : result.error ? (
        <div role="alert" className="space-y-3 p-5 text-sm">
          <p>Couldn't load this case: {result.error.message}</p>
          <Button variant="outline" size="sm" onClick={() => void result.refetch()}>
            Retry
          </Button>
        </div>
      ) : (
        <>
          {result.data.trials.length > 1 && (
            <div aria-label="Trials" className="flex flex-wrap items-center gap-1 border-b px-5 py-2">
              {result.data.trials.map((entry, index) => (
                <Button
                  key={entry.trial}
                  size="sm"
                  variant={index === trialIndex ? "secondary" : "ghost"}
                  aria-pressed={index === trialIndex}
                  onClick={() => setTrialIndex(index)}
                >
                  Trial {entry.trial}
                </Button>
              ))}
            </div>
          )}
          {trial ? (
            <TrialResult key={trial.trial} trial={trial} active={active} onBack={onBack} />
          ) : (
            <p role="status" className="p-5 text-sm text-muted-foreground">
              {active ? "Waiting for this case to finish" : "No trial results were recorded for this case"}
            </p>
          )}
        </>
      )}
    </section>
  );
}

function TrialResult({ trial, active, onBack }: { trial: TrialSteps; active: boolean; onBack: () => void }) {
  const [traceIndex, setTraceIndex] = useState(0);
  const trace = trial.traces?.[traceIndex];
  return (
    <>
      {trial.error && (
        <div
          role="alert"
          className="flex flex-wrap items-center gap-3 border-b bg-destructive/5 px-5 py-3 text-sm text-destructive"
        >
          <StatusBadge status="Error" />
          <span>{trial.error}</span>
        </div>
      )}
      {trial.checks.length > 0 && (
        <ul aria-label="Checks" className="flex flex-wrap gap-x-5 gap-y-2 border-b px-5 py-3 text-xs">
          {trial.checks.map((check) => (
            <li key={check.scorer} className="flex items-center gap-2">
              <StatusBadge status={check.passed ? "Passed" : "Failed"} />
              <span>{check.scorer}</span>
            </li>
          ))}
        </ul>
      )}
      {(trial.traces?.length ?? 0) > 1 && (
        <div aria-label="Case traces" className="flex flex-wrap items-center gap-1 border-b px-5 py-2">
          {trial.traces?.map((entry, index) => (
            <Button
              key={`${entry.trace_id}:${entry.trace_ref}`}
              size="sm"
              variant={index === traceIndex ? "secondary" : "ghost"}
              aria-pressed={index === traceIndex}
              onClick={() => setTraceIndex(index)}
            >
              Trace {index + 1}
            </Button>
          ))}
        </div>
      )}
      {trace ? (
        <CaseTrace key={`${trace.trace_id}:${trace.trace_ref}`} trace={trace} onBack={onBack} />
      ) : (
        <div className="space-y-2 p-5">
          <h4 className="text-sm font-medium">{active ? "Waiting for trace" : "Trace unavailable"}</h4>
          <p className="max-w-xl text-sm text-muted-foreground">
            {active
              ? "The trace will appear here when your agent uploads it"
              : "This trial has no linked trace. Run the eval with tracing enabled to inspect the agent's input, output, and steps"}
          </p>
        </div>
      )}
      {(trial.output || trial.steps.length > 0) && (
        <details className="border-t px-5 py-3 text-xs text-muted-foreground">
          <summary className="cursor-pointer select-none font-medium">Recorded output and steps</summary>
          {trial.output && (
            <pre className="mt-3 max-h-56 overflow-auto whitespace-pre-wrap rounded-md bg-muted/30 p-3 text-foreground">
              {trial.output}
            </pre>
          )}
          {trial.steps.length > 0 && (
            <ol aria-label="Recorded steps" className="mt-3 divide-y">
              {trial.steps.map((step, index) => (
                <li key={`${index}:${step.start_ns}`} className="flex gap-3 py-2">
                  <span className={cn("flex-1", !step.ok && "text-destructive")}>{step.tool_name || step.name}</span>
                  <span>{formatDuration(Math.max(0, step.end_ns - step.start_ns) / 1e6)}</span>
                </li>
              ))}
            </ol>
          )}
        </details>
      )}
    </>
  );
}

function CaseTrace({ trace, onBack }: { trace: TrialTrace; onBack: () => void }) {
  const accessToken = useLensAccessToken();
  const selection = useLocalRunSelection(null);
  return (
    <div className="flex min-h-[28rem] flex-1 flex-col">
      <RunView
        traceId={trace.trace_id}
        traceRef={trace.trace_ref}
        accessToken={accessToken}
        selection={selection}
        onBack={onBack}
        embedded
      />
    </div>
  );
}

function formatDuration(ms: number) {
  return ms < 1000 ? `${Math.round(ms)} ms` : `${(ms / 1000).toFixed(1)} s`;
}
