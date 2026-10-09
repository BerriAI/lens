"use client";

import { ArrowLeft, ChevronRight, Loader2, TriangleAlert } from "lucide-react";

import { StateMessage } from "../../../shared/StateMessage";
import { Button } from "../../../ui/button";
import { cn } from "../../../../lib/cva.config";

import { useEvalRun, useRunCases } from "./api";
import { CaseCompare } from "./CaseCompare";
import { shortSha } from "./format";
import { caseStatus, CriticalTag, RunStatusBadge, StatusBadge } from "./RunBadges";
import type { EvalRun, RunCaseSummary } from "./types";

export interface RunDetailProps {
  readonly runId: string;
  readonly caseId: string | null;
  readonly onBack: () => void;
  readonly onOpenCase: (caseId: string | null) => void;
}

export function RunDetail({ runId, caseId, onBack, onOpenCase }: RunDetailProps) {
  const run = useEvalRun(runId);
  if (run.isPending)
    return (
      <StateMessage
        role="status"
        icon={<Loader2 className="size-5 animate-spin motion-reduce:animate-none" />}
        title="Loading run…"
        description="Fetching its test results"
      />
    );
  if (run.error)
    return (
      <StateMessage
        role="alert"
        tone="destructive"
        icon={<TriangleAlert className="size-5" />}
        title="Couldn't load this run"
        description={run.error.message}
      >
        <Button size="sm" variant="outline" onClick={onBack}>
          All runs
        </Button>
      </StateMessage>
    );
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <RunHeader run={run.data} onBack={onBack} />
      <RunBody run={run.data} caseId={caseId} onOpenCase={onOpenCase} />
    </div>
  );
}

function RunHeader({ run, onBack }: { run: EvalRun; onBack: () => void }) {
  return (
    <header aria-label="Run" className="space-y-3 border-b px-5 py-4">
      <Button variant="ghost" size="sm" className="-ml-2 h-7 text-muted-foreground" onClick={onBack}>
        <ArrowLeft className="size-3.5" /> All runs
      </Button>
      <div className="flex flex-wrap items-center gap-3">
        <h2 className="min-w-0 truncate text-lg font-semibold">{run.eval}</h2>
        <RunStatusBadge run={run} />
        {run.summary && (
          <p aria-label="Case totals" className="ml-auto flex items-center gap-4 text-sm tabular-nums">
            <span className="text-success">{run.summary.passed} passed</span>
            <span className={cn(run.summary.total > run.summary.passed ? "text-destructive" : "text-muted-foreground")}>
              {Math.max(0, run.summary.total - run.summary.passed)} failed
            </span>
            <span className="text-muted-foreground">{run.summary.total} cases</span>
          </p>
        )}
      </div>
      <p className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
        <span>{run.branch}</span>
        <code>{shortSha(run.version)}</code>
        {run.pr !== null && <span>PR #{run.pr}</span>}
        <span>
          {run.received_trials}/{run.expected_trials} trials received
        </span>
      </p>
      {run.summary && !run.summary.gate.passed && (
        <div role="alert" aria-label="Run checks" className="space-y-1 text-xs text-destructive">
          <p>Run checks failed</p>
          {run.summary.gate.reasons.length > 0 && (
            <ul aria-label="Failed run checks" className="space-y-1">
              {run.summary.gate.reasons.map((reason) => (
                <li key={reason}>{reason}</li>
              ))}
            </ul>
          )}
        </div>
      )}
    </header>
  );
}

function RunBody({
  run,
  caseId,
  onOpenCase,
}: {
  run: EvalRun;
  caseId: string | null;
  onOpenCase: RunDetailProps["onOpenCase"];
}) {
  const active = run.status === "running" || run.status === "scoring";
  const cases = useRunCases(run.id, active);
  const selected = cases.data?.find((item) => item.case_id === caseId);
  return (
    <>
      {run.status === "failed" && (
        <p role="alert" className="border-b bg-destructive/5 px-5 py-3 text-sm text-destructive">
          {run.failure || "This run ended before it could finish scoring"}
        </p>
      )}
      {active && (
        <p role="status" className="border-b bg-muted/30 px-5 py-2 text-xs text-muted-foreground">
          {run.status === "running" ? "Running your agent" : "Scoring results"}. Results update automatically
        </p>
      )}
      {cases.isPending ? (
        <p role="status" className="px-5 py-6 text-sm text-muted-foreground">
          Loading cases…
        </p>
      ) : cases.error ? (
        <div role="alert" className="space-y-3 p-5 text-sm">
          <p>Couldn't load cases: {cases.error.message}</p>
          <Button variant="outline" size="sm" onClick={() => void cases.refetch()}>
            Retry
          </Button>
        </div>
      ) : cases.data.length === 0 ? (
        <p className="px-5 py-8 text-sm text-muted-foreground">
          {active ? "Waiting for the first case result" : "No case results were recorded for this run"}
        </p>
      ) : (
        <div className={cn("min-h-0 flex-1", caseId !== null && "flex flex-col lg:flex-row")}>
          <CaseList
            cases={cases.data}
            active={active}
            selected={caseId}
            onOpen={onOpenCase}
            compact={caseId !== null}
          />
          {caseId !== null && (
            <div className="flex min-w-0 flex-1 flex-col">
              {selected ? (
                <CaseCompare
                  key={`${run.id}:${caseId}`}
                  runId={run.id}
                  runStatus={run.status}
                  item={selected}
                  onBack={() => onOpenCase(null)}
                />
              ) : (
                <div role="alert" aria-label="Case unavailable" className="space-y-3 p-5 text-sm">
                  <p>Case {caseId} was not found in this run</p>
                  <Button variant="outline" size="sm" onClick={() => onOpenCase(null)}>
                    All cases
                  </Button>
                </div>
              )}
            </div>
          )}
        </div>
      )}
      <RunDiagnostics run={run} />
    </>
  );
}

function CaseList({
  cases,
  active,
  selected,
  onOpen,
  compact,
}: {
  cases: readonly RunCaseSummary[];
  active: boolean;
  selected: string | null;
  onOpen: (caseId: string) => void;
  compact: boolean;
}) {
  return (
    <nav
      aria-label="Test cases"
      className={cn("overflow-y-auto", compact && "shrink-0 border-b lg:w-72 lg:border-r lg:border-b-0")}
    >
      <div className="flex items-center border-b bg-muted/25 px-5 py-2.5 text-xs font-medium text-muted-foreground">
        <span className="flex-1">Test case</span>
        {!compact && <span className="w-28">Result</span>}
      </div>
      <ul>
        {cases.map((item) => (
          <li key={item.case_id}>
            <button
              type="button"
              onClick={() => onOpen(item.case_id)}
              aria-current={item.case_id === selected ? "true" : undefined}
              className={cn(
                "group flex w-full items-center gap-3 border-b px-5 py-3.5 text-left hover:bg-muted/40",
                selected === item.case_id && "bg-muted/50",
              )}
            >
              <span className="min-w-0 flex-1">
                <span className="flex items-center gap-2 text-sm">
                  <span className="truncate">{item.title || item.case_id}</span>
                  {item.critical && <CriticalTag />}
                </span>
                {compact && <StatusBadge status={caseStatus(item, active)} className="mt-1" />}
              </span>
              {!compact && <StatusBadge status={caseStatus(item, active)} className="w-24" />}
              <ChevronRight aria-hidden="true" className="size-3.5 shrink-0 text-muted-foreground" />
            </button>
          </li>
        ))}
      </ul>
    </nav>
  );
}

function RunDiagnostics({ run }: { run: EvalRun }) {
  return (
    <details className="mt-auto border-t px-5 py-3 text-xs text-muted-foreground">
      <summary className="cursor-pointer select-none font-medium">Run details</summary>
      <dl className="mt-3 grid grid-cols-[auto_1fr] gap-x-5 gap-y-2">
        <dt>Run ID</dt>
        <dd className="break-all font-mono">{run.id}</dd>
        <dt>Baseline</dt>
        <dd>{run.summary?.baseline_version ? shortSha(run.summary.baseline_version) : "No baseline"}</dd>
        {run.summary && (
          <>
            <dt>Regression policy</dt>
            <dd>
              {run.summary.gate.passed ? "Passed" : "Failed"}
              {run.summary.gate.reasons.length > 0 && (
                <ul className="mt-1 space-y-1" aria-label="Gate reasons">
                  {run.summary.gate.reasons.map((reason) => (
                    <li key={reason}>{reason}</li>
                  ))}
                </ul>
              )}
            </dd>
          </>
        )}
      </dl>
    </details>
  );
}
