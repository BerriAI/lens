"use client";

import {
  getCoreRowModel,
  useReactTable,
  type ColumnDef,
  type TableOptions,
} from "@tanstack/react-table";
import { useEffect, useRef } from "react";
import { Loader2, TriangleAlert } from "lucide-react";

import { Inspector } from "../../../shared/Inspector";
import { InspectorTable } from "../../../shared/InspectorTable";
import { StateMessage } from "../../../shared/StateMessage";
import { FINDING_PANEL_WIDTH_KEY } from "../../storage";

import { useEvalRuns } from "./api";
import { ConnectAgent } from "./ConnectAgent";
import { shortSha } from "./format";
import { RunStatusBadge } from "./RunBadges";
import { RunEval } from "./RunEval";
import type { EvalDefinition, EvalRun } from "./types";

export interface RunsTabProps {
  readonly definition: EvalDefinition;
  readonly datasetName: string;
  readonly revision: number;
  readonly onOpen: (runId: string) => void;
}

export function RunsTab({
  definition,
  datasetName,
  revision,
  onOpen,
}: RunsTabProps) {
  const runs = useEvalRuns({ eval: definition.name, include_ci: true });
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
  if (runs.data.length === 0)
    return (
      <div className="min-h-0 flex-1 overflow-y-auto">
        <ConnectAgent
          definition={definition}
          dataset={datasetName}
          revision={revision}
        />
      </div>
    );
  return (
    <>
      <RunEval
        definition={definition}
        runs={runs.data}
        onRefresh={runs.refetch}
      />
      <RunTable runs={runs.data} onOpen={onOpen} />
    </>
  );
}

const ROW_HEIGHT = 48;

const COLUMNS: ColumnDef<EvalRun>[] = [
  {
    id: "status",
    size: 140,
    header: "Status",
    cell: ({ row }) => <RunStatusBadge run={row.original} />,
  },
  {
    id: "commit",
    size: 280,
    header: "Branch / commit",
    cell: ({ row: { original: run } }) => (
      <span className="flex min-w-0 items-center gap-1 font-mono">
        <span className="truncate text-muted-foreground" title={run.branch}>{run.branch}</span>
        <span className="shrink-0">@{shortSha(run.version)}</span>
        {run.pr !== null && (
          <span className="ml-2 shrink-0 text-muted-foreground">#{run.pr}</span>
        )}
      </span>
    ),
  },
  {
    id: "results",
    header: "Cases",
    cell: ({ row: { original: run } }) =>
      run.summary ? (
        <span className="inline-flex items-center gap-3 tabular-nums">
          <span className="text-success">{run.summary.passed} passed</span>
          <span
            className={
              run.summary.total > run.summary.passed
                ? "text-destructive"
                : "text-muted-foreground"
            }
          >
            {run.summary.total - run.summary.passed} failed
          </span>
        </span>
      ) : (
        <span className="text-muted-foreground">
          {run.status === "failed"
            ? "No results"
            : `${run.received_trials}/${run.expected_trials} trials received`}
        </span>
      ),
  },
  {
    id: "run",
    size: 130,
    header: "Run",
    cell: ({ row }) => (
      <span className="font-mono text-muted-foreground" title={row.original.id}>
        {row.original.id.slice(-8)}
      </span>
    ),
  },
];

function RunTable({
  runs,
  onOpen,
}: {
  runs: readonly EvalRun[];
  onOpen: (id: string) => void;
}) {
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
        <div className="flex h-11 shrink-0 items-center border-b px-6 text-sm font-medium">
          Run history
        </div>
        <InspectorTable.Root table={table}>
          <InspectorTable.Grid
            aria-label="Eval runs"
            className="table-fixed text-sm"
            style={{ minWidth: 700 }}
          >
            <InspectorTable.Header />
            <InspectorTable.Body<EvalRun> rowHeight={() => ROW_HEIGHT}>
              {(row) => (
                <InspectorTable.Row
                  row={row}
                  item={row.original}
                  tabIndex={0}
                  aria-label={`${row.original.branch}@${shortSha(row.original.version)}`}
                  className="h-12"
                />
              )}
            </InspectorTable.Body>
          </InspectorTable.Grid>
        </InspectorTable.Root>
        <footer className="lens-toolbar flex h-8 shrink-0 items-center border-t px-3 font-mono text-[11px] text-muted-foreground">
          {runs.length} {runs.length === 1 ? "run" : "runs"}
        </footer>
      </div>
    </Inspector.Root>
  );
}
