"use client";

import { useEffect, useRef } from "react";
import { Loader2, TriangleAlert } from "lucide-react";

import { StateMessage } from "../../../shared/StateMessage";
import { cn } from "../../../../lib/cva.config";

import { useEvalRunRoute } from "../../route";
import { useEvalRuns } from "./api";
import { ConnectAgent } from "./ConnectAgent";
import { costPerCase, groupRuns, passedLabel, shortSha } from "./format";
import { GateStatus } from "./RunBadges";
import { RunDetail } from "./RunDetail";
import type { EvalRun } from "./types";

export interface RunsTabProps {
  readonly datasetId: string;
  readonly datasetName: string;
  readonly revision: number;
  readonly agentName: string;
}

export function RunsTab({
  datasetId,
  datasetName,
  revision,
  agentName,
}: RunsTabProps) {
  const { runId, caseId, openRun, openCase } = useEvalRunRoute();
  if (runId)
    return (
      <RunDetail
        key={runId}
        runId={runId}
        datasetId={datasetId}
        caseId={caseId}
        onBack={() => openRun(null)}
        onOpenCase={openCase}
      />
    );
  return (
    <RunList
      datasetId={datasetId}
      datasetName={datasetName}
      revision={revision}
      agentName={agentName}
      onOpen={openRun}
    />
  );
}

interface RunListProps extends RunsTabProps {
  readonly onOpen: (runId: string) => void;
}

function RunList({
  datasetId,
  datasetName,
  revision,
  agentName,
  onOpen,
}: RunListProps) {
  const runs = useEvalRuns({ agent: agentName, dataset: datasetId });
  const waited = useRef(false);
  const first = runs.data?.[0]?.id;
  useEffect(() => {
    if (runs.data?.length === 0) waited.current = true;
    else if (first && waited.current) {
      waited.current = false;
      onOpen(first);
    }
  }, [runs.data, first, onOpen]);
  if (runs.isPending)
    return (
      <StateMessage
        role="status"
        icon={
          <Loader2 className="size-5 animate-spin motion-reduce:animate-none" />
        }
        title="Loading runs…"
        description="Fetching eval runs for this dataset."
      />
    );
  if (runs.error)
    return (
      <StateMessage
        role="alert"
        tone="destructive"
        icon={<TriangleAlert className="size-5" />}
        title="Couldn't load runs"
        description={runs.error.message}
      />
    );
  const { main, pulls } = groupRuns(runs.data);
  if (main.length + pulls.length === 0)
    return (
      <ConnectAgent
        agent={agentName}
        dataset={datasetName}
        revision={revision}
      />
    );
  return (
    <div className="overflow-x-auto">
      <table className="w-full min-w-[860px] font-mono text-xs">
        <thead className="border-b bg-muted/30 text-[11px] text-muted-foreground">
          <tr>
            <th className={HEAD}>Gate</th>
            <th className={HEAD}>Commit</th>
            <th className={HEAD}>Eval</th>
            <th className={NUMERIC_HEAD}>Pass</th>
            <th className={NUMERIC_HEAD}>Regressed</th>
            <th className={NUMERIC_HEAD}>Fixed</th>
            <th className={NUMERIC_HEAD}>Cost/case</th>
            <th className={NUMERIC_HEAD}>Trials</th>
            <th className={HEAD}>Run</th>
          </tr>
        </thead>
        <RunGroup title="Pull requests" runs={pulls} onOpen={onOpen} />
        <RunGroup title="main" runs={main} onOpen={onOpen} />
      </table>
    </div>
  );
}

const HEAD = "px-3 py-1.5 text-left font-normal whitespace-nowrap";
const NUMERIC_HEAD = cn(HEAD, "text-right");
const CELL = "px-3 py-1.5 align-middle whitespace-nowrap";
const NUMERIC = cn(CELL, "text-right tabular-nums");

function RunGroup({
  title,
  runs,
  onOpen,
}: {
  title: string;
  runs: readonly EvalRun[];
  onOpen: (id: string) => void;
}) {
  if (runs.length === 0) return null;
  return (
    <tbody aria-label={title}>
      <tr className="border-b bg-muted/20">
        <th
          colSpan={9}
          scope="rowgroup"
          className="px-3 py-1 text-left text-[11px] font-normal text-muted-foreground uppercase"
        >
          {title}{" "}
          <span className="text-muted-foreground/70">{runs.length}</span>
        </th>
      </tr>
      {runs.map((run) => (
        <tr
          key={run.id}
          onClick={() => onOpen(run.id)}
          className="cursor-pointer border-b border-border/60 hover:bg-muted/40"
        >
          <td className={CELL}>
            <GateStatus run={run} />
          </td>
          <td className={CELL}>
            <button
              type="button"
              onClick={(event) => {
                event.stopPropagation();
                onOpen(run.id);
              }}
              className="rounded text-left text-foreground hover:underline focus-visible:outline-2 focus-visible:outline-ring"
            >
              <span className="text-muted-foreground">{run.branch}@</span>
              {shortSha(run.version)}
              {run.pr !== null && (
                <span className="ml-2 text-muted-foreground">#{run.pr}</span>
              )}
            </button>
          </td>
          <td className={cn(CELL, "text-muted-foreground")}>{run.eval}</td>
          <td className={NUMERIC}>
            {run.summary ? passedLabel(run.summary) : "–"}
          </td>
          <td
            className={cn(
              NUMERIC,
              run.summary?.regressions.length
                ? "text-destructive"
                : "text-muted-foreground",
            )}
          >
            {run.summary
              ? run.summary.baseline_run_id === null
                ? "n/a"
                : run.summary.regressions.length
              : "–"}
          </td>
          <td
            className={cn(
              NUMERIC,
              run.summary?.fixed.length
                ? "text-success"
                : "text-muted-foreground",
            )}
          >
            {run.summary
              ? run.summary.baseline_run_id === null
                ? "n/a"
                : run.summary.fixed.length
              : "–"}
          </td>
          <td className={NUMERIC}>
            {run.summary ? costPerCase(run.summary) : "–"}
          </td>
          <td className={cn(NUMERIC, "text-muted-foreground")}>
            {run.received_trials}/{run.expected_trials}
          </td>
          <td className={cn(CELL, "text-muted-foreground")}>
            {run.id.slice(0, 8)}
          </td>
        </tr>
      ))}
    </tbody>
  );
}
