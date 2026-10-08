"use client";

import { FlaskConical, Loader2, TriangleAlert } from "lucide-react";

import { StateMessage } from "@/components/shared/StateMessage";
import { cn } from "@/lib/cva.config";
import { formatActivityTimestamp } from "@/utils/activityTimestamp";

import { useEvalRunRoute } from "../../route";
import { useEvalRuns } from "./api";
import { costPerCase, deltaLabel, groupRuns, passedLabel, shortSha } from "./format";
import { GatePill, PullRequestLink } from "./RunBadges";
import { RunDetail } from "./RunDetail";
import type { EvalRun } from "./types";

export interface RunsTabProps {
  readonly datasetId: string;
}

export function RunsTab({ datasetId }: RunsTabProps) {
  const { runId, caseId, openRun, openCase } = useEvalRunRoute();
  if (runId)
    return <RunDetail key={runId} runId={runId} caseId={caseId} onBack={() => openRun(null)} onOpenCase={openCase} />;
  return <RunList datasetId={datasetId} onOpen={openRun} />;
}

const NO_FILTER = {};

function RunList({ datasetId, onOpen }: { datasetId: string; onOpen: (runId: string) => void }) {
  const runs = useEvalRuns(NO_FILTER);
  if (runs.isPending)
    return (
      <StateMessage
        role="status"
        icon={<Loader2 className="size-5 animate-spin motion-reduce:animate-none" />}
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
  const { main, pulls } = groupRuns(runs.data, datasetId);
  if (main.length + pulls.length === 0)
    return (
      <StateMessage
        role="status"
        icon={<FlaskConical className="size-5" />}
        title="No eval runs yet"
        description="Runs show up here once CI evaluates an agent against this dataset."
      />
    );
  return (
    <div className="flex flex-col gap-3 px-3 pt-3 pb-8 sm:px-4">
      <RunTable title="Main" runs={main} onOpen={onOpen} />
      <RunTable title="Pull requests" runs={pulls} onOpen={onOpen} />
    </div>
  );
}

const HEAD = "px-4 py-2 text-left text-xs font-normal text-muted-foreground";
const NUMERIC_HEAD = cn(HEAD, "text-right");
const CELL = "px-4 py-2.5 align-middle";
const NUMERIC = cn(CELL, "text-right tabular-nums");

function deltaTone(run: EvalRun): string {
  const delta = run.summary?.pass_rate_delta ?? null;
  if (delta === null || delta === 0) return "text-muted-foreground";
  return delta < 0 ? "text-destructive" : "text-success";
}

function RunTable({ title, runs, onOpen }: { title: string; runs: readonly EvalRun[]; onOpen: (id: string) => void }) {
  if (runs.length === 0) return null;
  return (
    <section aria-label={title} className="overflow-hidden rounded-xl border bg-card">
      <h3 className="px-4 pt-3 pb-1 text-sm font-medium text-foreground">{title}</h3>
      <div className="overflow-x-auto">
        <table className="w-full min-w-[720px] text-sm">
          <thead>
            <tr>
              <th className={HEAD}>Run</th>
              <th className={NUMERIC_HEAD}>Passed</th>
              <th className={NUMERIC_HEAD}>vs main</th>
              <th className={NUMERIC_HEAD}>Cost per case</th>
              <th className={HEAD}>Gate</th>
              <th className={HEAD}>
                <span className="sr-only">Pull request</span>
              </th>
            </tr>
          </thead>
          <tbody className="divide-y divide-border/60">
            {runs.map((run) => (
              <tr key={run.id} className="hover:bg-muted/40">
                <td className={CELL}>
                  <button
                    type="button"
                    onClick={() => onOpen(run.id)}
                    className="flex min-w-0 flex-col items-start rounded text-left focus-visible:outline-2 focus-visible:outline-ring"
                  >
                    <span className="font-medium text-foreground hover:underline">
                      Run <span className="font-mono">{shortSha(run.commit_sha)}</span> on {run.branch}
                    </span>
                    <span className="text-xs text-muted-foreground">
                      {run.eval} · revision {run.dataset_revision} · {formatActivityTimestamp(run.created_at)}
                    </span>
                  </button>
                </td>
                <td className={NUMERIC}>{run.summary ? passedLabel(run.summary) : "–"}</td>
                <td className={cn(NUMERIC, deltaTone(run))}>{run.summary ? deltaLabel(run.summary) : "–"}</td>
                <td className={NUMERIC}>{run.summary ? costPerCase(run.summary) : "–"}</td>
                <td className={CELL}>
                  <GatePill run={run} />
                </td>
                <td className={cn(CELL, "text-right")}>{run.pr_url && <PullRequestLink url={run.pr_url} />}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}
