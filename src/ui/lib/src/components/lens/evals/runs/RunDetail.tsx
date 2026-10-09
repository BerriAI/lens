"use client";

import { ArrowLeft, Loader2, TriangleAlert } from "lucide-react";

import { Inspector } from "../../../shared/Inspector";
import { Badge } from "../../../ui/badge";
import { useLensAccessToken } from "../../data/LensServices";
import { RunView } from "../../traces/detail/run/RunView";
import {
  traceKey,
  useOpenTraceRouting,
  type TraceRef,
} from "../../traces/routing";
import { StateMessage } from "../../../shared/StateMessage";
import { Button } from "../../../ui/button";
import { cn } from "../../../../lib/cva.config";

import { IdChip } from "../../traces/ui/IdChip";
import { useEvalRun, useRunCases } from "./api";
import { CaseCompare } from "./CaseCompare";
import { costPerCase, passedLabel, shortSha } from "./format";
import { CriticalTag, GateStatus } from "./RunBadges";
import type { CaseDiff, EvalRun, RunCaseSummary, Summary } from "./types";

export interface RunDetailProps {
  readonly runId: string;
  readonly caseId: string | null;
  readonly onBack: () => void;
  readonly onOpenCase: (caseId: string | null) => void;
}

export function RunDetail({
  runId,
  caseId,
  onBack,
  onOpenCase,
}: RunDetailProps) {
  const run = useEvalRun(runId);
  const accessToken = useLensAccessToken();
  const { trace, openTrace, selection, fullScreen, setFullScreen } =
    useOpenTraceRouting();
  if (run.isPending)
    return (
      <StateMessage
        role="status"
        icon={
          <Loader2 className="size-5 animate-spin motion-reduce:animate-none" />
        }
        title="Loading run…"
        description="Fetching its summary and gate."
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
    <Inspector.Root
      items={[]}
      itemKey={traceKey}
      selected={trace}
      onSelectedChange={openTrace}
      noun="eval trace"
      storageKey="litellm.evalTraces.drawerWidth"
      fullScreen={fullScreen}
      onFullScreenChange={setFullScreen}
    >
      <div className="flex min-h-0 flex-1 flex-col">
        <RunStrip run={run.data} onBack={onBack} />
        <RunBody run={run.data} caseId={caseId} onOpenCase={onOpenCase} />
        <Inspector.Panel label="Eval trace details">
          {(shown: TraceRef) => (
            <>
              <div className="flex items-center gap-2 border-b px-3 py-2 font-mono text-xs">
                <Badge variant="outline">Eval trace</Badge>
                <span className="truncate">{run.data.eval}</span>
              </div>
              <RunView
                traceId={shown.traceId}
                traceRef={shown.traceRef}
                selection={selection}
                accessToken={accessToken}
                onBack={() => openTrace(null)}
                embedded
              />
            </>
          )}
        </Inspector.Panel>
      </div>
    </Inspector.Root>
  );
}

const STRIP_ITEM =
  "flex items-center gap-1.5 border-l px-3 whitespace-nowrap first:border-l-0";
const KEY = "text-muted-foreground";

function RunStrip({ run, onBack }: { run: EvalRun; onBack: () => void }) {
  const summary = run.summary;
  return (
    <header
      aria-label="Run"
      className="lens-toolbar flex h-11 shrink-0 items-center overflow-x-auto border-b font-mono text-xs"
    >
      <button
        type="button"
        onClick={onBack}
        aria-label="All runs"
        className="flex h-full items-center gap-1 border-r px-3 text-muted-foreground hover:bg-muted hover:text-foreground"
      >
        <ArrowLeft className="size-3.5" />
        runs
      </button>
      <span className={STRIP_ITEM}>
        <GateStatus run={run} />
      </span>
      <span className={STRIP_ITEM}>{run.eval}</span>
      <span className={STRIP_ITEM}>
        <span className={KEY}>{run.branch}@</span>
        {shortSha(run.version)}
      </span>
      {run.pr !== null && (
        <span className={STRIP_ITEM}>
          <span className={KEY}>pr</span>#{run.pr}
        </span>
      )}
      <span className={STRIP_ITEM}>
        <span className={KEY}>vs</span>
        {summary?.baseline_version
          ? `main@${shortSha(summary.baseline_version)}`
          : "no baseline"}
      </span>
      {summary && (
        <>
          <span className={STRIP_ITEM}>
            <span className={KEY}>pass</span>
            {passedLabel(summary)}
          </span>
          <span
            className={cn(
              STRIP_ITEM,
              summary.regressions.length > 0 && "text-destructive",
            )}
          >
            <span className={KEY}>regressed</span>
            {summary.regressions.length}
          </span>
          <span
            className={cn(
              STRIP_ITEM,
              summary.fixed.length > 0 && "text-success",
            )}
          >
            <span className={KEY}>fixed</span>
            {summary.fixed.length}
          </span>
          <span className={STRIP_ITEM}>
            <span className={KEY}>cost</span>
            {costPerCase(summary)}/case
          </span>
        </>
      )}
      <span className={STRIP_ITEM}>
        <span className={KEY}>trials</span>
        {run.received_trials}/{run.expected_trials}
      </span>
      <span className={cn(STRIP_ITEM, "ml-auto")}>
        <IdChip value={run.id} label="Copy run ID" />
      </span>
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
  const cases = useRunCases(run.id);
  if (run.status === "failed")
    return (
      <p
        role="alert"
        className="border-b border-warning/40 bg-warning/5 px-3 py-2 font-mono text-xs text-warning"
      >
        error:{" "}
        {run.failure ||
          "This run ended with an error before it could be scored."}
      </p>
    );
  if (!run.summary)
    return (
      <p
        role="status"
        className="px-3 py-2 font-mono text-xs text-muted-foreground"
      >
        scoring… {run.received_trials}/{run.expected_trials} trials received
      </p>
    );
  const summary = run.summary;
  if (cases.isPending)
    return (
      <p role="status" className="px-3 py-3 text-xs text-muted-foreground">
        Loading cases…
      </p>
    );
  if (cases.error)
    return (
      <p role="alert" className="px-3 py-3 text-xs text-destructive">
        Couldn’t load cases: {cases.error.message}
      </p>
    );
  const requested = cases.data.find((item) => item.case_id === caseId) ?? null;
  const selected =
    requested ??
    (caseId === null
      ? cases.data.find(
          (item) => item.case_id === summary.regressions[0]?.case_id,
        ) ??
        cases.data[0] ??
        null
      : null);
  const change = summary.regressions.some(
    (item) => item.case_id === selected?.case_id,
  )
    ? "regressed"
    : summary.fixed.some((item) => item.case_id === selected?.case_id)
      ? "fixed"
      : "unchanged";
  return (
    <>
      <GateReasons summary={summary} />
      <div className="flex min-h-0 flex-1">
        <CaseList
          summary={summary}
          cases={cases.data}
          selected={selected?.case_id ?? null}
          onOpen={onOpenCase}
        />
        <div className="min-w-0 flex-1">
          {caseId !== null && requested === null ? (
            <MissingCase caseId={caseId} onDismiss={() => onOpenCase(null)} />
          ) : selected ? (
            <CaseCompare
              key={selected.case_id}
              diff={selected}
              runId={run.id}
              baselineRunId={summary.baseline_run_id}
              baselineVersion={summary.baseline_version}
              candidateVersion={run.version}
              change={change}
            />
          ) : (
            <p className="px-3 py-6 font-mono text-xs text-muted-foreground">
              No cases in this run.
            </p>
          )}
        </div>
      </div>
    </>
  );
}

function GateReasons({ summary }: { summary: Summary }) {
  if (summary.gate.reasons.length === 0) return null;
  return (
    <ul
      aria-label="Gate reasons"
      className={cn(
        "flex shrink-0 flex-wrap gap-x-4 border-b px-3 py-1.5 font-mono text-xs",
        summary.gate.passed
          ? "text-muted-foreground"
          : "bg-destructive/5 text-destructive",
      )}
    >
      {summary.gate.reasons.map((reason) => (
        <li key={reason}>✕ {reason}</li>
      ))}
    </ul>
  );
}

function CaseList({
  summary,
  cases,
  selected,
  onOpen,
}: {
  summary: Summary;
  cases: readonly RunCaseSummary[];
  selected: string | null;
  onOpen: (caseId: string) => void;
}) {
  return (
    <nav
      aria-label="Cases"
      className="w-72 shrink-0 overflow-y-auto border-r bg-[var(--lens-soft)]"
    >
      <CaseGroup
        title="Regressed"
        tone="text-destructive"
        diffs={summary.regressions}
        selected={selected}
        onOpen={onOpen}
      />
      <CaseGroup
        title="Fixed"
        tone="text-success"
        diffs={summary.fixed}
        selected={selected}
        onOpen={onOpen}
      />
      <CaseGroup
        title={summary.baseline_run_id === null ? "Cases" : "Unchanged"}
        tone="text-muted-foreground"
        diffs={cases.filter((item) => !findDiff(summary, item.case_id))}
        selected={selected}
        onOpen={onOpen}
      />
    </nav>
  );
}

function CaseGroup({
  title,
  tone,
  diffs,
  selected,
  onOpen,
}: {
  title: string;
  tone: string;
  diffs: readonly Pick<RunCaseSummary, "case_id" | "title" | "critical">[];
  selected: string | null;
  onOpen: (caseId: string) => void;
}) {
  if (diffs.length === 0) return null;
  return (
    <section aria-label={title}>
      <h4 className="lens-section-label flex items-center justify-between border-b px-3 py-2 font-mono text-[11px]">
        {title}
        <span className={tone}>{diffs.length}</span>
      </h4>
      <ul>
        {diffs.map((diff) => (
          <li key={diff.case_id}>
            <button
              type="button"
              aria-current={diff.case_id === selected ? "true" : undefined}
              onClick={() => onOpen(diff.case_id)}
              className={cn(
                "flex w-full items-start gap-2 border-b border-l-2 border-l-transparent px-3 py-1.5 text-left hover:bg-muted/50",
                diff.case_id === selected &&
                  "border-l-[var(--lens-brand)] bg-trace-row-selected",
              )}
            >
              <span
                aria-hidden="true"
                className={cn(
                  "mt-1.5 size-1.5 shrink-0 rounded-full bg-current",
                  tone,
                )}
              />
              <span className="min-w-0 flex-1">
                <span className="line-clamp-2 text-xs text-foreground">
                  {diff.title || diff.case_id}
                </span>
                <span className="flex items-center gap-1.5 font-mono text-[10px] text-muted-foreground">
                  {diff.case_id.slice(0, 12)}
                  {diff.critical && <CriticalTag />}
                </span>
              </span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}

function MissingCase({
  caseId,
  onDismiss,
}: {
  caseId: string;
  onDismiss: () => void;
}) {
  return (
    <div
      role="alert"
      className="flex items-center gap-3 border-b px-3 py-2 font-mono text-xs"
    >
      <TriangleAlert
        aria-hidden="true"
        className="size-3.5 shrink-0 text-warning"
      />
      <p className="min-w-0 flex-1 text-muted-foreground">
        Case <span className="text-foreground">{caseId}</span> was not found in
        this run.
      </p>
      <Button size="sm" variant="outline" onClick={onDismiss}>
        Dismiss
      </Button>
    </div>
  );
}

const findDiff = (summary: Summary, caseId: string): CaseDiff | null =>
  [...summary.regressions, ...summary.fixed].find(
    (diff) => diff.case_id === caseId,
  ) ?? null;
