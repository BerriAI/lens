"use client";

import { getCoreRowModel, useReactTable, type ColumnDef, type TableOptions } from "@tanstack/react-table";
import { useEffect, useRef } from "react";
import { Loader2, TriangleAlert } from "lucide-react";

import { Inspector } from "../../../shared/Inspector";
import { InspectorTable } from "../../../shared/InspectorTable";
import { StateMessage } from "../../../shared/StateMessage";
import { cn } from "../../../../lib/cva.config";
import { FINDING_PANEL_WIDTH_KEY } from "../../storage";

import { useEvalRuns } from "./api";
import { ConnectAgent } from "./ConnectAgent";
import { costPerCase, groupRuns, passedLabel, shortSha } from "./format";
import { GateStatus } from "./RunBadges";
import type { EvalDefinition, EvalRun } from "./types";

export interface RunsTabProps {
  readonly definition: EvalDefinition;
  readonly datasetName: string;
  readonly revision: number;
  readonly onOpen: (runId: string) => void;
}

export function RunsTab({ definition, datasetName, revision, onOpen }: RunsTabProps) {
  const runs = useEvalRuns({ eval: definition.name });
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
        icon={<Loader2 className="size-5 animate-spin motion-reduce:animate-none" />}
        title="Loading runs…"
        description="Fetching eval runs for this eval set."
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
      <div className="min-h-0 flex-1 overflow-y-auto">
        <ConnectAgent definition={definition} dataset={datasetName} revision={revision} />
      </div>
    );
  return <RunTable runs={[...pulls, ...main]} onOpen={onOpen} />;
}

const ROW_HEIGHT = 36;

const regressionCount = (run: EvalRun) =>
  run.summary ? (run.summary.baseline_run_id === null ? "n/a" : run.summary.regressions.length) : "–";
const fixedCount = (run: EvalRun) =>
  run.summary ? (run.summary.baseline_run_id === null ? "n/a" : run.summary.fixed.length) : "–";

const COLUMNS: ColumnDef<EvalRun>[] = [
  { id: "gate", size: 120, header: "Gate", cell: ({ row }) => <GateStatus run={row.original} /> },
  {
    id: "commit",
    size: 300,
    header: "Commit",
    cell: ({ row: { original: run } }) => (
      <span className="block min-w-0 truncate font-mono" title={`${run.branch}@${run.version}`}>
        <span className="text-muted-foreground">{run.branch}@</span>
        {shortSha(run.version)}
        {run.pr !== null && <span className="ml-2 text-muted-foreground">#{run.pr}</span>}
      </span>
    ),
  },
  {
    id: "agent",
    header: "Agent",
    cell: ({ row }) => <span className="truncate text-muted-foreground">{row.original.agent}</span>,
  },
  {
    id: "pass",
    size: 90,
    header: "Pass",
    meta: { numeric: true },
    cell: ({ row }) => (row.original.summary ? passedLabel(row.original.summary) : "–"),
  },
  {
    id: "regressed",
    size: 100,
    header: "Regressed",
    meta: { numeric: true },
    cell: ({ row }) => (
      <span className={cn(row.original.summary?.regressions.length ? "text-destructive" : "text-muted-foreground")}>
        {regressionCount(row.original)}
      </span>
    ),
  },
  {
    id: "fixed",
    size: 80,
    header: "Fixed",
    meta: { numeric: true },
    cell: ({ row }) => (
      <span className={cn(row.original.summary?.fixed.length ? "text-success" : "text-muted-foreground")}>
        {fixedCount(row.original)}
      </span>
    ),
  },
  {
    id: "cost",
    size: 100,
    header: "Cost/case",
    meta: { numeric: true },
    cell: ({ row }) => (row.original.summary ? costPerCase(row.original.summary) : "–"),
  },
  {
    id: "trials",
    size: 80,
    header: "Trials",
    meta: { numeric: true },
    cell: ({ row }) => `${row.original.received_trials}/${row.original.expected_trials}`,
  },
  {
    id: "run",
    size: 100,
    header: "Run",
    cell: ({ row }) => <span className="font-mono text-muted-foreground">{row.original.id.slice(0, 8)}</span>,
  },
];

function RunTable({ runs, onOpen }: { runs: readonly EvalRun[]; onOpen: (id: string) => void }) {
  const tableOptions: TableOptions<EvalRun> = {
    data: [...runs],
    columns: COLUMNS,
    defaultColumn: { size: undefined },
    getRowId: (run) => run.id,
    autoResetAll: false,
    getCoreRowModel: getCoreRowModel(),
  };
  const table = useReactTable(tableOptions);
  return (
    <Inspector.Root
      items={runs}
      itemKey={(run) => run.id}
      selected={null}
      onSelectedChange={(run) => run && onOpen(run.id)}
      noun="run"
      storageKey={FINDING_PANEL_WIDTH_KEY}
    >
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
        <InspectorTable.Root table={table}>
          <InspectorTable.Grid aria-label="Eval runs" className="text-xs" style={{ minWidth: 960 }}>
            <InspectorTable.Header />
            <InspectorTable.Body<EvalRun> rowHeight={() => ROW_HEIGHT}>
              {(row) => (
                <InspectorTable.Row
                  row={row}
                  item={row.original}
                  tabIndex={0}
                  aria-label={`${row.original.branch}@${shortSha(row.original.version)}`}
                  className="h-9"
                />
              )}
            </InspectorTable.Body>
          </InspectorTable.Grid>
        </InspectorTable.Root>
        <footer className="flex h-8 shrink-0 items-center border-t bg-muted/30 px-3 text-xs text-muted-foreground">
          {runs.length} {runs.length === 1 ? "run" : "runs"}
        </footer>
      </div>
    </Inspector.Root>
  );
}
