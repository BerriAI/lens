"use client";

import { useState } from "react";
import { getCoreRowModel, useReactTable, type ColumnDef, type TableOptions } from "@tanstack/react-table";
import { ChevronLeft, FlaskConical, Loader2, Plus, TriangleAlert } from "lucide-react";

import { Inspector } from "../../shared/Inspector";
import { InspectorTable } from "../../shared/InspectorTable";
import { StateMessage } from "../../shared/StateMessage";
import { Button } from "../../ui/button";
import { useDatasets } from "../datasets/api";
import type { DatasetSummary } from "../datasets/types";
import { useEvalRunRoute } from "../route";
import { FINDING_PANEL_WIDTH_KEY } from "../storage";
import { LensPageHeader } from "../ui/LensPageHeader";
import { NewEval } from "./NewEval";
import { useEvalDefinition, useEvals } from "./runs/api";
import { RunDetail } from "./runs/RunDetail";
import { RunsTab } from "./runs/RunsTab";
import type { EvalDefinition } from "./runs/types";
import { gateLabel, scorerLabel } from "./labels";

export function EvalsView() {
  const { evalName, runId, caseId, openEval, openRun, openCase } = useEvalRunRoute();
  const [creating, setCreating] = useState(false);
  if (runId)
    return (
      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto bg-card">
        <RunDetail key={runId} runId={runId} caseId={caseId} onBack={() => openRun(null)} onOpenCase={openCase} />
      </div>
    );
  if (creating)
    return (
      <NewEval
        onCancel={() => setCreating(false)}
        onSaved={(name) => {
          setCreating(false);
          openEval(name);
        }}
      />
    );
  if (evalName) return <EvalPage key={evalName} name={evalName} onBack={() => openEval(null)} onOpenRun={openRun} />;
  return (
    <>
      <LensPageHeader
        section="06 / REGRESSION LAB"
        title="Evals"
        description="Replay production. Catch regressions before they ship."
        actions={
          <Button size="sm" onClick={() => setCreating(true)}>
            <Plus /> New eval
          </Button>
        }
      />
      <EvalList onOpen={openEval} />
    </>
  );
}

const loading = (title: string) => (
  <StateMessage
    role="status"
    icon={<Loader2 className="size-5 animate-spin motion-reduce:animate-none" />}
    title={title}
    description="This takes a moment."
  />
);

const failed = (title: string, error: Error) => (
  <StateMessage
    role="alert"
    tone="destructive"
    icon={<TriangleAlert className="size-5" />}
    title={title}
    description={error.message}
  />
);

const datasetLabel = (definition: EvalDefinition, datasets: readonly DatasetSummary[]) => {
  const dataset = datasets.find((item) => item.id === definition.spec.dataset_id);
  const name = dataset?.name ?? definition.spec.dataset_id;
  return `${name}@${definition.spec.revision ?? dataset?.revision ?? "latest"}`;
};

function EvalList({ onOpen }: { onOpen: (name: string) => void }) {
  const evals = useEvals();
  const datasets = useDatasets();
  if (evals.isPending) return loading("Loading evals…");
  if (evals.error) return failed("Couldn't load evals", evals.error);
  if (evals.data.length === 0)
    return (
      <div className="lens-empty-state flex min-h-0 flex-1 flex-col items-center justify-center gap-3">
        <StateMessage
          role="status"
          icon={<FlaskConical className="size-5" />}
          title="No evals yet"
          description="An eval replays a dataset of production cases against every PR and fails the check on regressions."
        />
      </div>
    );
  return <EvalTable evals={evals.data} datasets={datasets.data ?? []} onOpen={onOpen} />;
}

function EvalTable({
  evals,
  datasets,
  onOpen,
}: {
  evals: readonly EvalDefinition[];
  datasets: readonly DatasetSummary[];
  onOpen: (name: string) => void;
}) {
  const columns: ColumnDef<EvalDefinition>[] = [
    {
      id: "name",
      size: 220,
      header: "Eval",
      cell: ({ row }) => (
        <span className="flex min-w-0 items-center gap-2.5 font-mono">
          <span className="flex size-7 shrink-0 items-center justify-center rounded-md border border-[var(--lens-violet)]/20 bg-[var(--lens-violet)]/10 text-[var(--lens-violet)]">
            <FlaskConical aria-hidden="true" className="size-3.5" />
          </span>
          <span className="truncate">{row.original.name}</span>
        </span>
      ),
    },
    {
      id: "agent",
      size: 160,
      header: "Agent",
      cell: ({ row }) => <span className="font-mono">{row.original.spec.agent}</span>,
    },
    {
      id: "dataset",
      size: 200,
      header: "Dataset",
      cell: ({ row }) => (
        <span className="font-mono text-muted-foreground">{datasetLabel(row.original, datasets)}</span>
      ),
    },
    {
      id: "scorers",
      header: "Scorers",
      cell: ({ row }) => (
        <span className="truncate font-mono text-muted-foreground">
          {row.original.spec.scorers.map(scorerLabel).join(", ")}
        </span>
      ),
    },
    {
      id: "trials",
      size: 70,
      header: "Trials",
      meta: { numeric: true },
      cell: ({ row }) => row.original.spec.trials,
    },
    {
      id: "gate",
      size: 200,
      header: "Gate",
      cell: ({ row }) => <span className="font-mono text-muted-foreground">{gateLabel(row.original.spec.gate)}</span>,
    },
  ];
  const tableOptions: TableOptions<EvalDefinition> = {
    data: [...evals],
    columns,
    defaultColumn: { size: undefined },
    getRowId: (definition) => definition.name,
    autoResetAll: false,
    getCoreRowModel: getCoreRowModel(),
  };
  const table = useReactTable(tableOptions);
  return (
    <Inspector.Root
      items={evals}
      itemKey={(definition) => definition.name}
      selected={null}
      onSelectedChange={(definition) => definition && onOpen(definition.name)}
      noun="eval"
      storageKey={FINDING_PANEL_WIDTH_KEY}
    >
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden bg-card">
        <div className="lens-toolbar flex h-9 shrink-0 items-center gap-2 border-b px-3">
          <span aria-hidden="true" className="size-1.5 rounded-full bg-[var(--lens-violet)]" />
          <span className="font-mono text-[11px] text-muted-foreground">
            {evals.length} {evals.length === 1 ? "eval" : "evals"}
          </span>
        </div>
        <InspectorTable.Root table={table}>
          <InspectorTable.Grid aria-label="Evals" className="lens-table text-xs" style={{ minWidth: 960 }}>
            <InspectorTable.Header />
            <InspectorTable.Body<EvalDefinition> rowHeight={() => 36}>
              {(row) => (
                <InspectorTable.Row
                  row={row}
                  item={row.original}
                  tabIndex={0}
                  aria-label={row.original.name}
                  className="h-9"
                />
              )}
            </InspectorTable.Body>
          </InspectorTable.Grid>
        </InspectorTable.Root>
      </div>
    </Inspector.Root>
  );
}

function EvalPage({
  name,
  onBack,
  onOpenRun,
}: {
  name: string;
  onBack: () => void;
  onOpenRun: (runId: string) => void;
}) {
  const definition = useEvalDefinition(name);
  const datasets = useDatasets();
  if (definition.isPending) return loading("Loading eval…");
  if (definition.error) return failed("Couldn't load eval", definition.error);
  const spec = definition.data.spec;
  const dataset = datasets.data?.find((item) => item.id === spec.dataset_id);
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden bg-card">
      <div className="lens-toolbar flex min-h-12 shrink-0 flex-wrap items-center gap-3 border-b px-3 py-2 text-xs">
        <Button variant="ghost" size="xs" onClick={onBack}>
          <ChevronLeft /> Evals
        </Button>
        <h2 className="flex shrink-0 items-center gap-2 whitespace-nowrap font-mono font-medium">
          <FlaskConical aria-hidden="true" className="size-4 text-[var(--lens-violet)]" />
          {name}
        </h2>
        <dl className="flex min-w-0 items-center gap-3 truncate font-mono text-muted-foreground">
          <dt className="sr-only">Agent</dt>
          <dd>{spec.agent}</dd>
          <dt className="sr-only">Dataset</dt>
          <dd>{datasetLabel(definition.data, datasets.data ?? [])}</dd>
          <dt className="sr-only">Scorers</dt>
          <dd className="truncate">{spec.scorers.map(scorerLabel).join(", ")}</dd>
          <dt className="sr-only">Trials</dt>
          <dd>{spec.trials}× trials</dd>
          <dt className="sr-only">Gate</dt>
          <dd>{gateLabel(spec.gate)}</dd>
        </dl>
      </div>
      <RunsTab
        definition={definition.data}
        datasetName={dataset?.name ?? spec.dataset_id}
        revision={spec.revision ?? dataset?.revision ?? 1}
        onOpen={onOpenRun}
      />
    </div>
  );
}
