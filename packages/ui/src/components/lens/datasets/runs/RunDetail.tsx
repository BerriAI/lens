"use client";

import { ChevronRight, Loader2, TriangleAlert } from "lucide-react";

import { StateMessage } from "../../../shared/StateMessage";
import { Button } from "../../../ui/button";
import { cn } from "../../../../lib/cva.config";

import { IdChip } from "../../traces/ui/IdChip";
import { useEvalRun } from "./api";
import { CaseCompare } from "./CaseCompare";
import { costPerCase, deltaLabel, deltaTone, passedLabel, shortSha } from "./format";
import { CriticalPill, GatePill, PullRequestLink } from "./RunBadges";
import type { CaseDiff, EvalRun, Summary } from "./types";

export interface RunDetailProps {
  readonly runId: string;
  readonly datasetId: string;
  readonly caseId: string | null;
  readonly onBack: () => void;
  readonly onOpenCase: (caseId: string | null) => void;
}

export function RunDetail({ runId, datasetId, caseId, onBack, onOpenCase }: RunDetailProps) {
  const run = useEvalRun(runId);
  if (run.isPending)
    return (
      <StateMessage
        role="status"
        icon={<Loader2 className="size-5 animate-spin motion-reduce:animate-none" />}
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
  if (run.data.dataset_id !== datasetId)
    return (
      <StateMessage
        role="alert"
        icon={<TriangleAlert className="size-5" />}
        title="This run belongs to another dataset"
        description={`Run ${run.data.id} evaluated dataset ${run.data.dataset_id}.`}
      >
        <Button size="sm" variant="outline" onClick={onBack}>
          All runs
        </Button>
      </StateMessage>
    );
  return (
    <div className="flex flex-col gap-3 px-3 pt-3 pb-8 sm:px-4">
      <RunHeader run={run.data} onBack={onBack} />
      <RunBody run={run.data} caseId={caseId} onOpenCase={onOpenCase} />
    </div>
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
  if (run.status === "error")
    return (
      <p role="alert" className="py-10 text-center text-sm text-muted-foreground">
        This run ended with an error before it could be scored.
      </p>
    );
  if (!run.summary)
    return (
      <p role="status" className="py-10 text-center text-sm text-muted-foreground">
        This run is still being scored.
      </p>
    );
  const diff = findDiff(run.summary, caseId);
  return (
    <>
      <SummaryStrip summary={run.summary} />
      <GateReasons run={run} />
      {diff ? (
        <CaseCompare key={diff.case_id} diff={diff} onClose={() => onOpenCase(null)} />
      ) : (
        <>
          {caseId && <MissingCase caseId={caseId} onDismiss={() => onOpenCase(null)} />}
          <DiffList title="Regressions" diffs={run.summary.regressions} onOpen={onOpenCase} />
          <DiffList title="Fixed" diffs={run.summary.fixed} onOpen={onOpenCase} />
        </>
      )}
    </>
  );
}

function MissingCase({ caseId, onDismiss }: { caseId: string; onDismiss: () => void }) {
  return (
    <div role="alert" className="flex items-center gap-3 rounded-xl border bg-card px-4 py-3 text-sm">
      <TriangleAlert aria-hidden="true" className="size-4 shrink-0 text-warning" />
      <p className="min-w-0 flex-1 text-muted-foreground">
        Case <span className="font-mono text-foreground">{caseId}</span> did not regress or get fixed in this run.
      </p>
      <Button size="sm" variant="outline" onClick={onDismiss}>
        Dismiss
      </Button>
    </div>
  );
}

const findDiff = (summary: Summary, caseId: string | null): CaseDiff | null =>
  caseId === null ? null : [...summary.regressions, ...summary.fixed].find((diff) => diff.case_id === caseId) ?? null;

function RunHeader({ run, onBack }: { run: EvalRun; onBack: () => void }) {
  return (
    <header className="flex flex-col gap-1">
      <nav aria-label="Run breadcrumb" className="flex items-center gap-1 text-xs text-muted-foreground">
        <button type="button" onClick={onBack} className="rounded hover:text-foreground hover:underline">
          Runs
        </button>
        <ChevronRight aria-hidden="true" className="size-3" />
        <span className="truncate">{run.eval}</span>
      </nav>
      <div className="flex flex-wrap items-center gap-2">
        <h3 className="text-sm font-semibold text-foreground">
          Run <span className="font-mono">{shortSha(run.commit_sha)}</span> on {run.branch}
        </h3>
        <IdChip value={run.id} label="Copy run ID" />
        <GatePill run={run} />
        <span className="text-xs text-muted-foreground">
          {run.agent} · revision {run.dataset_revision}
        </span>
        <span className="ml-auto">
          <PullRequestLink url={run.pr_url} />
        </span>
      </div>
    </header>
  );
}

function Stat({ label, value, note, tone }: { label: string; value: string; note: string; tone?: string }) {
  return (
    <div className="px-4 pt-3.5 pb-3">
      <dt className="text-xs text-muted-foreground">{label}</dt>
      <dd className={cn("mt-1 text-xl font-semibold tracking-tight tabular-nums", tone)}>{value}</dd>
      <dd className="text-xs text-muted-foreground">{note}</dd>
    </div>
  );
}

function SummaryStrip({ summary }: { summary: Summary }) {
  return (
    <dl
      aria-label="Run summary"
      className="grid grid-cols-2 divide-border rounded-xl border bg-card lg:grid-cols-4 lg:divide-x"
    >
      <Stat
        label="Passed"
        value={passedLabel(summary)}
        note={`${summary.failed} failed · ${summary.errored} errored`}
      />
      <Stat
        label="vs baseline"
        value={deltaLabel(summary)}
        note={summary.baseline_reason ?? "Pass rate against the baseline"}
        tone={summary.pass_rate_delta ? deltaTone(summary.pass_rate_delta) : undefined}
      />
      <Stat
        label="Regressions"
        value={String(summary.regressions.length)}
        note={`${summary.fixed.length} fixed`}
        tone={summary.regressions.length ? "text-destructive" : undefined}
      />
      <Stat label="Cost per case" value={costPerCase(summary)} note="Average across all cases" />
    </dl>
  );
}

function GateReasons({ run }: { run: EvalRun }) {
  if (!run.gate || run.gate.reasons.length === 0) return null;
  return (
    <section aria-label="Gate reasons" className="rounded-xl border bg-card px-4 pt-3 pb-3">
      <h4 className="text-sm font-medium text-foreground">Why the gate {run.gate.passed ? "passed" : "failed"}</h4>
      <ul className="mt-1.5 flex flex-col gap-1 text-sm text-muted-foreground">
        {run.gate.reasons.map((reason, index) => (
          <li key={index}>{reason}</li>
        ))}
      </ul>
    </section>
  );
}

function DiffList({
  title,
  diffs,
  onOpen,
}: {
  title: string;
  diffs: readonly CaseDiff[];
  onOpen: (caseId: string) => void;
}) {
  return (
    <section aria-label={title} className="rounded-xl border bg-card">
      <h4 className="flex items-center gap-2 px-4 pt-3 pb-1 text-sm font-medium text-foreground">
        {title}
        <span className="inline-flex min-w-5 justify-center rounded-full bg-muted px-1.5 text-xs tabular-nums text-foreground">
          {diffs.length}
        </span>
      </h4>
      {diffs.length === 0 ? (
        <p className="px-4 pt-1 pb-3 text-sm text-muted-foreground">None</p>
      ) : (
        <ul className="divide-y divide-border/60">
          {diffs.map((diff) => (
            <li key={diff.case_id}>
              <button
                type="button"
                onClick={() => onOpen(diff.case_id)}
                className="flex w-full min-w-0 items-center gap-3 px-4 py-2.5 text-left text-sm hover:bg-muted/40 focus-visible:outline-2 focus-visible:outline-ring"
              >
                <span className="min-w-0 flex-1 truncate text-foreground">{diff.input || diff.case_id}</span>
                {diff.critical && <CriticalPill />}
                <ChevronRight aria-hidden="true" className="size-3.5 shrink-0 text-muted-foreground" />
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
